#![allow(dead_code)]
#![allow(unused)]
#![feature(duration_constructors)]

// NOTE: I want to be able to work under the assumption that a valid session_id always exists in
// all observable ways, with the exception of ensure_session_id, which ensures that invariant.

// NOTE: I've observed some weird behavior with the ensure_session_id middleware, it seems to get
// called multiple times in a burst when getting called from a mobile browser. Maybe not a problem?
// Investigation needed.

// TODO: Add `Session` extractor that acts like `CookieJar` pass it like a resource and get it like
// a resource. Very nice and rusty :P. :Basicaly, reimplement tower_sessions, but diferent.

use anyhow::anyhow;
use axum::{
    async_trait,
    extract::{
        ConnectInfo, FromRef, FromRequestParts, Host, OriginalUri, Path, Query, Request, State,
    },
    handler::HandlerWithoutStateExt,
    http::{request::Parts, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, IntoResponseParts, Redirect, Response, ResponseParts},
    routing::get,
    Extension, Json, RequestExt, Router, ServiceExt,
};
use axum_extra::{
    extract::{
        cookie::{self, Cookie, Expiration, SameSite},
        CookieJar,
    },
    middleware::option_layer,
};
use extract::SessionId;
use itertools::Itertools;
use jotdown::Render;
use lazy_static::lazy_static;
use minijinja::{context, path_loader, value::ViaDeserialize};
use notify::Watcher;
use oauth2::{
    basic::{BasicClient, BasicErrorResponseType},
    reqwest::async_http_client,
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, RedirectUrl, RevocationUrl,
    Scope, StandardErrorResponse, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashMap,
    convert::Infallible,
    env, fs,
    hash::{DefaultHasher, Hasher},
    net::SocketAddr,
    ops::{Deref, DerefMut},
    path::{self, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Instant,
};
use time::OffsetDateTime;
use tokio::{
    net::TcpListener,
    sync::{Mutex, RwLock},
};
use tower::{Layer, ServiceBuilder};
use tower_http::{
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
    timeout::TimeoutLayer,
};
use tower_livereload::LiveReloadLayer;
use tracing::{info, level_filters::LevelFilter, span, trace, warn, Level};
use tracing_subscriber::{
    filter::EnvFilter,
    fmt::{self, time::FormatTime},
    layer::SubscriberExt,
};
use tracing_subscriber::{filter::FilterFn, util::SubscriberInitExt};
use uuid::{uuid, Uuid};
use walkdir::WalkDir;

use qed_core::{Repository, User};

mod extract;
mod infra;

use crate::infra::libsql::LibsqlRepository;

struct Config {
    oauth_google_client_id: String,
    oauth_google_client_secret: String,

    turso_url: String,
    turso_token: String,

    port: String,
    debug: bool,
}

type PageStore = HashMap<Uuid, Document>;

struct App {
    reloader: minijinja_autoreload::AutoReloader,
    google_auth_client: BasicClient,
    db: Arc<libsql::Database>,
    repository: Arc<Mutex<LibsqlRepository>>,
    documents: Arc<RwLock<PageStore>>,
}

pub trait LibsqlValueRefExt {
    fn to_value<'a>(&'a self) -> libsql::ValueRef<'a>;
}

impl LibsqlValueRefExt for Uuid {
    fn to_value<'a>(&'a self) -> libsql::ValueRef<'a> {
        libsql::ValueRef::Blob(self.as_bytes())
    }
}

impl LibsqlValueRefExt for String {
    fn to_value<'a>(&'a self) -> libsql::ValueRef<'a> {
        libsql::ValueRef::Text(self.as_bytes())
    }
}

#[async_trait]
trait SessionStore: Send + Sync {
    type Id;
    type Record;
    type Error;

    async fn is_valid(&self, id: Self::Id) -> Result<bool, Self::Error>;
    async fn get(&self, id: Self::Id) -> Result<Self::Record, Self::Error>;
    async fn set(&self, id: Self::Id, record: Self::Record) -> Result<(), Self::Error>;
    async fn register(&self) -> Result<Self::Id, Self::Error>;
    async fn deregister(&self, id: Self::Id) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SessionRecord {
    csrf_state: Option<String>,
    user_id: Option<Uuid>,
}

impl From<SessionRecord> for libsql::Value {
    fn from(val: SessionRecord) -> Self {
        libsql::Value::Text(serde_json::to_string(&val).unwrap())
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for SessionRecord
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let SessionId(id): SessionId = parts
            .extensions
            .get()
            .cloned()
            .expect("session id must be set");

        let session_store: Arc<
            dyn SessionStore<Id = Uuid, Record = SessionRecord, Error = SessionError>,
        > = parts
            .extensions
            .get()
            .cloned()
            .expect("session store must be present, forgot to add?");

        Ok(session_store
            .get(id)
            .await
            .expect("ensure_session_id should make a valid id"))
    }
}

impl IntoResponseParts for SessionRecord {
    type Error = Infallible;

    fn into_response_parts(
        self,
        mut res: ResponseParts,
    ) -> Result<axum::response::ResponseParts, Self::Error> {
        res.extensions_mut().insert(self);
        dbg!(&res);

        Ok(res)
    }
}

#[derive(thiserror::Error, Debug)]
enum SessionError {
    #[error("session not found")]
    SessionNotFound,

    #[error(transparent)]
    Libsql(#[from] libsql::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Clone)]
struct LibsqlSessionStore {
    db: Arc<libsql::Database>,
}

impl LibsqlSessionStore {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SessionStore for LibsqlSessionStore {
    type Id = Uuid;
    type Record = SessionRecord;
    type Error = SessionError;

    async fn is_valid(&self, id: Self::Id) -> Result<bool, Self::Error> {
        let conn = self.db.connect()?;

        let mut rows = conn
            .query(
                "SELECT count(*) FROM sessions WHERE id = ?1",
                [id.as_bytes().to_vec()],
            )
            .await?;

        let row = rows.next().await?.unwrap();

        Ok(row.get::<i32>(0)? == 1)
    }

    async fn get(&self, id: Self::Id) -> Result<Self::Record, Self::Error> {
        let conn = self.db.connect()?;

        let mut rows = conn
            .query(
                "SELECT data FROM sessions WHERE id = ?1",
                [id.as_bytes().as_ref()],
            )
            .await?;

        let row = rows.next().await?.ok_or(SessionError::SessionNotFound)?;

        use libsql::Value as V;
        match row.get_value(0)? {
            V::Null => Ok(Default::default()),
            V::Text(text) => Ok(serde_json::from_str(text.as_str())?),
            _ => panic!("data field should be either null or text"),
        }
    }

    async fn set(&self, id: Self::Id, record: Self::Record) -> Result<(), Self::Error> {
        let conn = self.db.connect()?;

        let mut rows_changed = conn
            .execute(
                "UPDATE sessions SET data = ?1 WHERE id = ?2",
                libsql::params![record, id.as_bytes().as_ref()],
            )
            .await?;

        assert!(rows_changed <= 1);

        Ok(())
    }

    async fn register(&self) -> Result<Self::Id, Self::Error> {
        let conn = self.db.connect()?;
        let id = Uuid::now_v7();

        info!("created another session {id}", id = id);

        let mut rows_changed = conn
            .execute(
                "INSERT INTO sessions (id) VALUES (?1)",
                libsql::params![id.as_bytes().to_vec()],
            )
            .await?;

        assert!(rows_changed == 1);
        Ok(id)
    }

    async fn deregister(&self, id: Self::Id) -> Result<(), Self::Error> {
        let conn = self.db.connect()?;

        info!("deleted session {id}", id = id);

        let mut rows_changed = conn
            .execute(
                "DELETE FROM sessions WHERE id = ?1",
                libsql::params![id.as_bytes().to_vec()],
            )
            .await?;

        assert!(rows_changed == 1);

        Ok(())
    }
}

type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error(transparent)]
    OauthUrlParse(#[from] oauth2::url::ParseError),

    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),

    #[error(transparent)]
    EnvVar(#[from] env::VarError),

    #[error(transparent)]
    Minijinja(#[from] minijinja::Error),

    #[error(transparent)]
    Notify(#[from] notify::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Libsql(#[from] libsql::Error),

    #[error(transparent)]
    De(#[from] serde::de::value::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),

    #[error(transparent)]
    Session(#[from] SessionError),

    #[error(transparent)]
    LibsqlRepository(#[from] qed_core::RepositoryError<infra::libsql::Error>),

    #[error("Invalid CSRF state, found `{found}` (expected `{expected}`)")]
    InvalidCsrf { expected: String, found: String },
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(format!("<pre>{:?}</pre>", self)),
        )
            .into_response()
    }
}

fn host_to_callback(Host(host): Host, path: impl AsRef<str>) -> String {
    let path = path.as_ref();

    if host.starts_with("localhost:") {
        format!("http://{host}{path}")
    } else {
        format!("https://{host}{path}")
    }
}

fn parse<E: std::error::Error>(
    path: impl AsRef<path::Path>,
    repo: &mut impl qed_core::Repository<E>,
) -> Document {
    use jotdown::{Container as C, Event as E};

    let input = String::from_utf8(fs::read(path.as_ref()).unwrap()).unwrap();
    let mut events = jotdown::Parser::new(input.as_ref()).collect::<Vec<_>>();
    let mut toml: Option<String> = None;

    for event in &events {
        match event {
            E::Start(C::RawBlock { format: "toml" }, _) => {
                toml = Some(String::new());
            }
            E::End(C::RawBlock { format: "toml" }) => {
                if toml.is_some() {
                    break;
                }
            }
            E::Str(s) => {
                if let Some(ref mut toml) = toml {
                    toml.push_str(s);
                }
            }
            _ => {}
        }
    }

    let mut metadata = toml::from_str::<Metadata>(toml.unwrap().as_ref()).unwrap();
    let mut test_tags = metadata.tags.clone();

    events
        .iter_mut()
        .filter_map(|event| {
            if let E::Start(C::Div { class: "question" }, ref mut attrs) = event {
                Some(attrs)
            } else {
                None
            }
        })
        .zip(0..)
        .for_each(|(attrs, count)| {
            if let Some(tags) = attrs.get("data-tags") {
                let mut tags = tags
                    .to_string()
                    .split(',')
                    .filter(|e| !e.is_empty())
                    .map(|e| e.to_string())
                    .collect::<Vec<String>>();

                let mut q_tags = test_tags
                    .iter()
                    .chain(tags.iter())
                    .cloned()
                    .collect::<Vec<String>>();

                q_tags.sort();
                q_tags.dedup();
                q_tags.sort_by_key(|e| e.len());

                repo.add_question(&qed_core::Document { id: metadata.uuid }, count, q_tags);

                metadata.tags.append(&mut tags.clone());
            }

            attrs.insert("data-position", format!("{}", count).into());
            attrs.insert(
                "data-id",
                format!(
                    "{}",
                    Uuid::new_v5(
                        &qed_core::NAMESPACE_QUESTION,
                        format!("{}.{}", metadata.uuid, count).as_bytes(),
                    )
                )
                .into(),
            );
            attrs.insert("data-test", format!("{}", metadata.uuid).into());
            attrs.insert("id", format!("q{}", count).into());
        });

    metadata.tags.sort();
    metadata.tags.dedup();
    metadata.tags.sort_by_key(|e| e.len());

    let mut html = String::new();
    jotdown::html::Renderer::default()
        .push(events.into_iter(), &mut html)
        .unwrap();

    Document {
        html,
        metadata,
        path: path.as_ref().to_owned(),
        timestamp: Instant::now(),
    }
}

#[axum_macros::debug_handler]
async fn document(
    State(app): State<Arc<App>>,
    Path(uuid): Path<Uuid>,
    extract::User(user): extract::User,
) -> Result<impl IntoResponse> {
    let docs = app.documents.read().await;
    let document = docs.get(&uuid).unwrap();

    let env = app.reloader.acquire_env().unwrap();
    let home_template = env.get_template("document.html")?;

    Ok(Html(home_template.render(context! {
        content => document.html,
        user_picture => user.map(|user| user.picture),
    })?))
}

async fn document_list(
    State(app): State<Arc<App>>,
    extract::User(user): extract::User,
) -> Result<impl IntoResponse> {
    let docs = app.documents.read().await;
    let repo = app.repository.lock().await;

    let env = app.reloader.acquire_env().unwrap();
    let home_template = env.get_template("document_list.html")?;

    Ok(Html(home_template.render(context! {
        documents => docs.values().collect::<Vec<_>>(),
        user_picture => user.map(|user| user.picture),
    })?))
}

#[derive(Serialize, Deserialize, Debug)]
struct Metadata {
    uuid: Uuid,
    title: String,
    tags: Vec<String>,
}

#[derive(Serialize, Debug)]
struct Document {
    html: String,
    metadata: Metadata,
    path: PathBuf,
    #[serde(skip)]
    timestamp: Instant,
}

#[derive(Deserialize, Debug)]
pub struct Params {
    code: String,
    state: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Claims {
    sub: Uuid,
    exp: u64,
}

async fn google_callback(
    State(app): State<Arc<App>>,
    Query(params): Query<Params>,
    SessionId(id): SessionId,
    record: SessionRecord,
    host: Host,
    req: Request,
) -> Result<impl IntoResponse> {
    let code = AuthorizationCode::new(params.code);
    let expected_state = record.csrf_state.clone().unwrap_or("".to_string());

    if expected_state != params.state {
        return Err(Error::InvalidCsrf {
            expected: expected_state.to_owned(),
            found: params.state,
        });
    }

    let token_result = app
        .google_auth_client
        .clone()
        .set_redirect_uri(RedirectUrl::new(host_to_callback(
            host,
            "/oauth/google/callback",
        ))?)
        .exchange_code(code)
        .request_async(&async_http_client)
        .await
        // TODO: fix this shit, this error type is the definition of evil.
        .map_err(|err| anyhow!("{:?}", err))?;

    let google_user: qed_core::GoogleUser = reqwest::Client::new()
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(token_result.access_token().secret())
        .send()
        .await?
        .json()
        .await?;

    let auth = qed_core::Auth::GoogleOauth(google_user.clone());

    let mut repo = app.repository.lock().await;

    use qed_core::RepositoryError as UE;

    let user = match repo.register_user(auth).await {
        Ok(user) => user,
        Err(UE::UserAlreadyExists(auth)) => User::from_auth(auth, repo.deref()).await?,
        Err(err) => return Err(err.into()),
    };

    Ok((
        SessionRecord {
            user_id: Some(user.id),
            csrf_state: None,
            ..record
        },
        Redirect::to("/d"),
    ))
}

async fn login(
    State(app): State<Arc<App>>,
    SessionId(id): SessionId,
    record: SessionRecord,
    host: Host,
) -> Result<impl IntoResponse> {
    let (authorize_url, csrf_state) = app
        .google_auth_client
        .clone()
        .set_redirect_uri(RedirectUrl::new(host_to_callback(
            host,
            "/oauth/google/callback",
        ))?)
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("profile".to_string()))
        .add_scope(Scope::new("email".to_string()))
        .add_scope(Scope::new("openid".to_string()))
        .url();

    Ok((
        SessionRecord {
            csrf_state: Some(csrf_state.secret().to_owned()),
            ..record
        },
        Redirect::to(authorize_url.as_str()),
    ))
}

async fn ensure_session_id(
    State(app): State<Arc<App>>,
    jar: CookieJar,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    mut request: Request,
    next: Next,
) -> Result<Response> {
    let span = span!(Level::INFO, "ensure_session_id");
    let guard = span.enter();

    let session_store = request
        .extensions()
        .get::<Arc<dyn SessionStore<Id = Uuid, Record = SessionRecord, Error = SessionError>>>()
        .cloned()
        .expect("session_store must exist");

    let id = jar
        .get("id")
        .and_then(|cookie| Uuid::from_str(cookie.value()).ok());

    info!("current session {id:?} for {addr:?}", id = id, addr = addr);

    let jar = match id {
        Some(id) if session_store.is_valid(id).await? => {
            request.extensions_mut().insert(SessionId(id));
            jar
        }
        _ => {
            let jar = jar.remove(Cookie::from("id"));
            let id = session_store.register().await?;
            request.extensions_mut().insert(SessionId(id));

            info!(
                "session avaiable {id:?}",
                id = request.extensions_mut().get::<SessionId>().unwrap().0
            );

            let cookie = Cookie::build(Cookie::new("id", id.to_string()))
                .secure(true)
                .http_only(true)
                .same_site(SameSite::Lax)
                .expires(Expiration::DateTime(
                    OffsetDateTime::now_utc() + time::Duration::days(5),
                ))
                .path("/")
                .build();

            jar.add(cookie)
        }
    };

    let SessionId(id) = request
        .extensions_mut()
        .get::<SessionId>()
        .cloned()
        .unwrap();

    let response = next.run(request).await;

    if let Some(record) = response.extensions().get::<SessionRecord>() {
        session_store.set(id.to_owned(), record.to_owned()).await?;
    }

    Ok((jar, response).into_response())
}
async fn logout(
    State(app): State<Arc<App>>,
    SessionId(id): SessionId,
    jar: CookieJar,
    record: SessionRecord,
) -> Result<impl IntoResponse> {
    Ok((
        SessionRecord {
            user_id: None,
            ..record
        },
        Redirect::to("/d/"),
    ))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer().with_thread_names(true))
        .with(LevelFilter::INFO)
        .with(FilterFn::new(|metadata| {
            !matches!(
                metadata.target(),
                "libsql::replication::remote_client" | "libsql_replication::replicator",
            )
        }))
        .init();

    if dotenv::dotenv().is_err() {
        warn!("running without .env");
    }

    let config = Config {
        oauth_google_client_id: env::var("OAUTH_GOOGLE_CLIENT_ID")?,
        oauth_google_client_secret: env::var("OAUTH_GOOGLE_CLIENT_SECRET")?,
        turso_url: env::var("TURSO_URL")?,
        turso_token: env::var("TURSO_TOKEN")?,
        port: "4000".to_string(),
        debug: env::var("ENV") == Ok("".to_string()),
    };

    let db = Arc::new(
        libsql::Builder::new_remote_replica("replica.db", config.turso_url, config.turso_token)
            .sync_interval(std::time::Duration::from_secs(1))
            .read_your_writes(true)
            .build()
            .await?,
    );

    let repo = Arc::new(Mutex::new(LibsqlRepository::new(db.clone()).await?));
    let docs = Arc::new(RwLock::new(PageStore::new()));

    for entry in WalkDir::new("content/")
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "djot"))
    {
        let page = parse(entry.path(), repo.lock().await.deref_mut());

        docs.write().await.insert(page.metadata.uuid, page);
    }

    let mut content_notifier = notify::recommended_watcher({
        let repo = Arc::clone(&repo);
        let docs = Arc::clone(&docs);
        move |event: Result<notify::Event, notify::Error>| {
            if let notify::Event {
                kind: notify::EventKind::Modify(_),
                paths,
                attrs: _,
            } = event.unwrap()
            {
                for page in docs.blocking_write().values_mut() {
                    if paths.contains(
                        &fs::canonicalize(&page.path).expect("content dir should be present"),
                    ) {
                        *page = parse(page.path.clone(), repo.blocking_lock().deref_mut());
                    }
                }
            }
        }
    })?;

    content_notifier.watch(
        path::Path::new("content/"),
        notify::RecursiveMode::Recursive,
    )?;
    content_notifier.watch(
        path::Path::new("templates/"),
        notify::RecursiveMode::Recursive,
    )?;
    content_notifier.watch(
        path::Path::new("assets/tw.css"),
        notify::RecursiveMode::NonRecursive,
    )?;

    let mut reloader = minijinja_autoreload::AutoReloader::new(move |notifier| {
        let path = "templates/";

        let mut env = minijinja::Environment::new();
        env.set_loader(path_loader(path));
        env.add_function(
            "tag_color",
            |s: ViaDeserialize<String>, l: ViaDeserialize<f32>| -> String {
                let mut hasher = DefaultHasher::new();
                hasher.write(s.as_bytes());
                let n = hasher.finish();

                let a = (n.wrapping_shr(8 + 1) & 0xff) as u8;
                let b = (n.wrapping_shr(2) & 0xff) as u8;
                let lr = ((n & 0b1111) as i8 - 0b1000) as f32;

                let r = 0.5;
                let offset = r / 2.;

                let a = (a as f32 / u8::MAX as f32) * r - offset;
                let b = (b as f32 / u8::MAX as f32) * r - offset;

                let c = oklab::oklab_to_srgb(oklab::Oklab {
                    l: l.0 + lr * 0.01,
                    a,
                    b,
                });

                format!("{}", c)
            },
        );

        notifier.watch_path(path, true);

        Ok(env)
    });

    let state = Arc::from(App {
        db,
        repository: repo,
        documents: docs,
        reloader,
        google_auth_client: {
            let client_id = ClientId::new(config.oauth_google_client_id);
            let client_secret = ClientSecret::new(config.oauth_google_client_secret);
            let auth_url =
                AuthUrl::new("https://accounts.google.com/o/oauth2/v2/auth".to_string())?;
            let token_url =
                TokenUrl::new("https://www.googleapis.com/oauth2/v3/token".to_string())?;

            BasicClient::new(client_id, Some(client_secret), auth_url, Some(token_url))
                .set_revocation_uri(RevocationUrl::new(
                    "https://oauth2.googleapis.com/revoke".to_string(),
                )?)
        },
    });

    let app = Router::new()
        .fallback_service((StatusCode::NOT_FOUND, Html("404")).into_service())
        .nest_service("/favicon.ico", ServeFile::new("assets/qed.ico"))
        .nest_service("/assets", ServeDir::new("assets"))
        .route("/oauth/google/callback", get(google_callback))
        .route("/login", get(login))
        .route("/logout", get(logout))
        .route("/d", get(|| async { Redirect::permanent("/d/") }))
        .route("/d/", get(document_list))
        .route("/d/:uuid", get(document))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            ensure_session_id,
        ))
        // .layer(CatchPanicLayer::new())
        .layer(Extension::<
            Arc<dyn SessionStore<Id = Uuid, Record = SessionRecord, Error = SessionError>>,
        >(Arc::from(LibsqlSessionStore::new(Arc::clone(
            &state.db,
        )))))
        .layer(CompressionLayer::new().br(true))
        .layer(TimeoutLayer::new(std::time::Duration::from_secs(1)))
        .with_state(Arc::clone(&state));

    let listener = TcpListener::bind(format!("0.0.0.0:{port}", port = config.port)).await?;

    info!("{:?}", listener);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
