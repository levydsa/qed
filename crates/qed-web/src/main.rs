#![allow(dead_code)]
#![allow(unused)]
#![feature(duration_constructors)]

use anyhow::anyhow;
use axum::{
    async_trait,
    extract::{ConnectInfo, FromRef, FromRequestParts, Host, OriginalUri, Path, Query, Request, State},
    handler::HandlerWithoutStateExt,
    http::{request::Parts, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Extension, Json, RequestExt, Router, ServiceExt,
};
use axum_extra::extract::{
    cookie::{self, Cookie, Expiration, SameSite},
    CookieJar,
};
use extractors::SessionId;
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
    collections::HashMap, convert::Infallible, env, fs, hash::{DefaultHasher, Hasher}, net::SocketAddr, ops::{Deref, DerefMut}, path::{self, PathBuf}, str::FromStr, sync::Arc, time::Instant
};
use time::OffsetDateTime;
use tokio::{
    net::TcpListener,
    sync::{Mutex, RwLock},
};
use tower::ServiceBuilder;
use tower_http::{
    timeout::TimeoutLayer,
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
};
use tower_livereload::LiveReloadLayer;
use tracing::{info, level_filters::LevelFilter, span, trace, warn, Level};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{
    filter::EnvFilter,
    fmt::{self, time::FormatTime},
    layer::SubscriberExt,
};
use uuid::{uuid, Uuid};
use walkdir::WalkDir;

use qed_core::Repository;

mod extractors;
mod infra;

use crate::extractors::User;
use crate::infra::libsql::LibsqlRepository;
use crate::infra::mem::MemoryRepository;

struct Config {
    oauth_google_client_id: String,
    oauth_google_client_secret: String,

    turso_url: String,
    turso_token: String,

    hmac_key: String,
}

type PageStore = HashMap<Uuid, Document>;

struct App {
    reloader: minijinja_autoreload::AutoReloader,
    google_auth_client: BasicClient,
    db: Arc<libsql::Database>,
    repository: Arc<Mutex<LibsqlRepository>>,
    documents: Arc<RwLock<PageStore>>,
}

pub trait LibsqlValueExt {
    fn to_value<'a>(&'a self) -> libsql::ValueRef<'a>;
}

impl LibsqlValueExt for Uuid {
    fn to_value<'a>(&'a self) -> libsql::ValueRef<'a> {
        libsql::ValueRef::Blob(self.as_bytes())
    }
}

impl LibsqlValueExt for String {
    fn to_value<'a>(&'a self) -> libsql::ValueRef<'a> {
        libsql::ValueRef::Text(self.as_bytes())
    }
}

#[async_trait]
trait SessionStore {
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

struct LibsqlSessionStore(libsql::Connection);

#[async_trait]
impl SessionStore for LibsqlSessionStore {
    type Id = Uuid;
    type Record = SessionRecord;
    type Error = SessionError;

    async fn is_valid(&self, id: Self::Id) -> Result<bool, Self::Error> {
        let mut rows = self
            .0
            .query(
                "SELECT count(*) FROM sessions WHERE id = ?1",
                [id.as_bytes().to_vec()],
            )
            .await?;

        let row = rows.next().await?.unwrap();

        Ok(row.get::<i32>(0)? == 1)
    }

    async fn get(&self, id: Self::Id) -> Result<Self::Record, Self::Error> {
        let mut rows = self
            .0
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
        let mut rows_changed = self
            .0
            .execute(
                "UPDATE sessions SET data = ?1 WHERE id = ?2",
                libsql::params![record, id.as_bytes().as_ref()],
            )
            .await?;

        assert!(rows_changed <= 1);

        Ok(())
    }

    async fn register(&self) -> Result<Self::Id, Self::Error> {
        let id = Uuid::now_v7();

        info!("created another session {id}", id = id);

        let mut rows_changed = self
            .0
            .execute(
                "INSERT INTO sessions (id) VALUES (?1)",
                libsql::params![id.as_bytes().to_vec()],
            )
            .await?;

        assert!(rows_changed == 1);
        Ok(id)
    }

    async fn deregister(&self, id: Self::Id) -> Result<(), Self::Error> {
        info!("deleted session {id}", id = id);

        let mut rows_changed = self
            .0
            .execute(
                "DELETE FROM sessions WHERE id = ?1",
                libsql::params![id.as_bytes().to_vec()],
            )
            .await?;

        assert!(rows_changed == 1);

        Ok(())
    }
}

const CSRF_STATE_KEY: &str = "csrf_state";

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
    MemoryRepository(#[from] infra::mem::Error),

    #[error(transparent)]
    LibsqlRepository(#[from] infra::libsql::Error),

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

fn parse(path: impl AsRef<path::Path>, repo: &mut impl qed_core::Repository) -> Document {
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

type Result<T, E = Error> = std::result::Result<T, E>;

#[axum_macros::debug_handler]
async fn document(
    State(app): State<Arc<App>>,
    Path(uuid): Path<Uuid>,
    User(user): extractors::User,
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
    User(user): extractors::User,
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
    host: Host,
    req: Request,
) -> Result<impl IntoResponse> {
    let session_store = LibsqlSessionStore(app.db.connect()?);

    let code = AuthorizationCode::new(params.code);

    let record = session_store.get(id).await?;

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

    use qed_core::UserError as UE;

    let user = repo.register_user(auth.clone()).await?;
    let user = match user {
        Ok(user) => user,
        Err(err) => match err {
            UE::UserAlreadyExists => repo
                .get_user_from_auth(auth.clone())
                .await?
                .expect("if user already exists, this is unreachable"),
            UE::UserNotFound => unreachable!(),
        },
    };

    session_store
        .set(
            id,
            SessionRecord {
                user_id: Some(user.id),
                ..record
            },
        )
        .await?;

    Ok(Redirect::to("/d"))
}

async fn login(
    State(app): State<Arc<App>>,
    SessionId(id): SessionId,
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

    let session_store = LibsqlSessionStore(app.db.connect()?);
    let record = session_store.get(id).await?;
    session_store
        .set(
            id,
            SessionRecord {
                csrf_state: Some(csrf_state.secret().to_owned()),
                ..record
            },
        )
        .await?;

    Ok(Redirect::to(authorize_url.as_str()))
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

    let session_store = LibsqlSessionStore(app.db.connect()?);

    let id = jar
        .get("id")
        .and_then(|cookie| Uuid::from_str(cookie.value()).ok());

    info!("current session {id:?} for {addr:?}", id = id, addr = addr);

    match id {
        Some(id) if session_store.is_valid(id).await? => {
            request.extensions_mut().insert(SessionId(id));
            Ok((next.run(request).await).into_response())
        }
        Some(_) => {
            warn!("found a invalid session for {addr:?}", addr = addr);
            let jar = jar.remove(Cookie::from("id"));
            Ok((jar, next.run(request).await).into_response())
        }
        _ => {
            let jar = jar.remove(Cookie::from("id"));
            let id = session_store.register().await?;
            request.extensions_mut().insert(SessionId(id));

            info!("session avaiable {id:?}", id = request.extensions_mut().get::<SessionId>().unwrap().0);


            let cookie = Cookie::build(Cookie::new("id", id.to_string()))
                .secure(true)
                .http_only(true)
                .same_site(SameSite::Lax)
                .expires(Expiration::DateTime(
                    OffsetDateTime::now_utc() + time::Duration::days(5),
                ))
                .path("/")
                .build();

            Ok((jar.add(cookie), next.run(request).await).into_response())
        }
    }
}
async fn logout(
    State(app): State<Arc<App>>,
    SessionId(id): SessionId,
    jar: CookieJar,
) -> Result<impl IntoResponse> {
    let session_store = LibsqlSessionStore(app.db.connect()?);
    session_store.deregister(id).await?;
    let cookie = jar.get("id").unwrap().clone();
    Ok((jar.remove(cookie), Redirect::to("/d/")))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer().with_thread_names(true))
        .with(EnvFilter::from_default_env())
        .init();

    if dotenv::dotenv().is_err() {
        info!("running without .env");
    }

    let config = Config {
        oauth_google_client_id: env::var("OAUTH_GOOGLE_CLIENT_ID")?,
        oauth_google_client_secret: env::var("OAUTH_GOOGLE_CLIENT_SECRET")?,
        turso_url: env::var("TURSO_URL")?,
        turso_token: env::var("TURSO_TOKEN")?,
        hmac_key: env::var("HMAC_KEY")?,
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

    let livereload = LiveReloadLayer::new();

    let mut content_notifier = notify::recommended_watcher({
        let repo = Arc::clone(&repo);
        let docs = Arc::clone(&docs);
        let reloader = livereload.reloader();
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
                reloader.reload()
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
            state.clone(),
            ensure_session_id,
        ))
        .layer(
            livereload.request_predicate(|req: &Request| !req.headers().contains_key("hx-request")),
        )
        // .layer(CatchPanicLayer::new())
        .layer(CompressionLayer::new().br(true))
        .layer(TimeoutLayer::new(std::time::Duration::from_secs(1)))
        .with_state(state.clone());

    let listener = TcpListener::bind("0.0.0.0:4000").await?;

    info!("{:?}", listener);

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;

    Ok(())
}
