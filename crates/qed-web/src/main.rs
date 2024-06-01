#![allow(dead_code)]
#![allow(unused)]
#![feature(duration_constructors)]

use std::{
    collections::HashMap,
    convert::Infallible,
    env, fs,
    hash::{DefaultHasher, Hasher},
    ops::DerefMut,
    path::{self, PathBuf},
    sync::Arc,
    time::{self, Instant},
};

use jwt::DecodingKey;
use qed_core::Repository;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path, Query, Request, State},
    handler::HandlerWithoutStateExt,
    http::{request::Parts, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Extension, Json, Router,
};
use axum_extra::extract::{
    cookie::{self, Cookie, SameSite},
    CookieJar,
};
use itertools::Itertools;
use jotdown::Render;
use jsonwebtoken as jwt;
use lazy_static::lazy_static;
use minijinja::{context, path_loader, value::ViaDeserialize};
use notify::Watcher;
use oauth2::{
    basic::BasicClient, reqwest::async_http_client, AuthUrl, AuthorizationCode, ClientId,
    ClientSecret, CsrfToken, RedirectUrl, RevocationUrl, Scope, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use tokio::{
    net::TcpListener,
    sync::{Mutex, RwLock},
};
use tower_http::{
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
};
use tower_livereload::LiveReloadLayer;
use tracing::{info, level_filters::LevelFilter, warn};
use uuid::{uuid, Uuid};
use walkdir::WalkDir;

mod extractors;
mod infra;

use crate::extractors::User;
use crate::infra::MemoryRepository;

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error(transparent)]
    Libcore(#[from] qed_core::Error),

    #[error(transparent)]
    Jwt(#[from] jwt::errors::Error),

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
    Other(#[from] anyhow::Error),

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

fn tag_color(s: ViaDeserialize<String>, l: ViaDeserialize<f32>) -> String {
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
) -> Result<impl IntoResponse> {
    let docs = app.documents.read().await;
    let document = docs.get(&uuid).unwrap();

    let env = app.reloader.acquire_env().unwrap();
    let home_template = env.get_template("base.html")?;

    Ok(Html(
        home_template.render(context!(content => document.html))?,
    ))
}

async fn documents(
    State(app): State<Arc<App>>,
    Extension(dec_key): Extension<Arc<jwt::DecodingKey>>,
    Extension(header): Extension<Arc<jwt::Header>>,
    User(user): extractors::User,
) -> Result<impl IntoResponse> {
    let docs = app.documents.read().await;
    let repo = app.repository.lock().await;

    let env = app.reloader.acquire_env().unwrap();
    let home_template = env.get_template("documents.html")?;

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
    Query(params): Query<Params>,
    State(app): State<Arc<App>>,
    Extension(enc_key): Extension<Arc<jwt::EncodingKey>>,
    Extension(header): Extension<Arc<jwt::Header>>,
    jar: CookieJar,
) -> Result<impl IntoResponse> {
    let mut repo = app.repository.lock().await;

    let code = AuthorizationCode::new(params.code);

    let expected_state = jar
        .get("csrf_state")
        .map(|cookie| cookie.value())
        .unwrap_or("");

    if expected_state != params.state {
        return Err(Error::InvalidCsrf {
            expected: expected_state.to_owned(),
            found: params.state,
        });
    }

    let token_result = app
        .google_auth_client
        .exchange_code(code)
        .request_async(&async_http_client)
        .await
        .map_err(|e| anyhow::anyhow!("{:?}", e))?;

    let google_user: qed_core::GoogleUser = reqwest::Client::new()
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(token_result.access_token().secret())
        .send()
        .await?
        .json()
        .await?;

    let auth = qed_core::Auth::GoogleOauth(google_user.clone());

    let user = match repo.register_user(auth.clone()).await {
        Ok(user) => user,
        Err(qed_core::Error::UserAlreadyRegistered) => repo.get_user_from_auth(auth).await.unwrap(),
        Err(_) => unreachable!(),
    };

    Ok((
        jar.add(
            Cookie::build((
                "auth",
                jwt::encode(
                    &header,
                    &Claims {
                        sub: user.id,
                        exp: jwt::get_current_timestamp()
                            + tokio::time::Duration::from_days(3).as_secs(),
                    },
                    &enc_key,
                )?,
            ))
            .same_site(SameSite::Lax)
            .secure(true)
            .http_only(true)
            .path("/")
            .build(),
        ),
        Redirect::to("/d/"),
    ))
}

async fn login(State(app): State<Arc<App>>, jar: CookieJar) -> impl IntoResponse {
    let (authorize_url, csrf_state) = app
        .google_auth_client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("profile".to_string()))
        .add_scope(Scope::new("email".to_string()))
        .add_scope(Scope::new("openid".to_string()))
        .url();

    (
        jar.add(
            Cookie::build(("csrf_state", csrf_state.secret().to_owned()))
                .same_site(SameSite::Lax)
                .secure(true)
                .http_only(true)
                .build(),
        ),
        Redirect::to(authorize_url.as_str()),
    )
}

type PageStore = HashMap<Uuid, Document>;

struct App {
    reloader: minijinja_autoreload::AutoReloader,
    google_auth_client: BasicClient,
    repository: Arc<Mutex<MemoryRepository>>,
    documents: Arc<RwLock<PageStore>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .without_time()
        .with_max_level(LevelFilter::INFO)
        .compact()
        .init();

    if dotenv::dotenv().is_err() {
        info!("running without .env");
    }

    let repo = Arc::from(Mutex::new(MemoryRepository::new()));
    let docs = Arc::from(RwLock::new(PageStore::new()));

    for entry in WalkDir::new("content/")
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "djot"))
    {
        let page = parse(entry.path(), repo.lock().await.deref_mut());

        dbg!(&page.metadata.uuid);

        docs.write().await.insert(page.metadata.uuid, page);
    }

    let livereload = LiveReloadLayer::new();
    let reloader = livereload.reloader();

    let mut repo_ref = repo.clone();
    let mut docs_ref = docs.clone();

    let mut content_notifier =
        notify::recommended_watcher(move |event: Result<notify::Event, notify::Error>| {
            if let notify::Event {
                kind: notify::EventKind::Modify(_),
                paths,
                attrs: _,
            } = event.unwrap()
            {
                for page in docs_ref.blocking_write().values_mut() {
                    if paths.contains(
                        &fs::canonicalize(&page.path).expect("content dir should be present"),
                    ) {
                        *page = parse(page.path.clone(), repo_ref.blocking_lock().deref_mut());
                    }
                }
                reloader.reload()
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
        env.add_function("tag_color", tag_color);

        notifier.watch_path(path, true);

        Ok(env)
    });

    let app = Router::new()
        .fallback_service(Html("We are fucked OMG!!!").into_service())
        .route("/login", get(login))
        .route("/d/", get(documents))
        .route("/d/:uuid", get(document))
        .nest(
            "/oauth",
            Router::new().route("/google/callback", get(google_callback)),
        )
        .nest_service("/favicon.ico", ServeFile::new("assets/qed.ico"))
        .nest_service("/assets", ServeDir::new("assets"))
        .with_state(Arc::from(App {
            repository: repo,
            documents: docs,
            reloader,
            google_auth_client: {
                let google_client_id = ClientId::new(env::var("OAUTH_GOOGLE_CLIENT_ID")?);
                let google_client_secret =
                    ClientSecret::new(env::var("OAUTH_GOOGLE_CLIENT_SECRET")?);

                let auth_url =
                    AuthUrl::new("https://accounts.google.com/o/oauth2/v2/auth".to_string())?;
                let token_url =
                    TokenUrl::new("https://www.googleapis.com/oauth2/v3/token".to_string())?;

                BasicClient::new(
                    google_client_id,
                    Some(google_client_secret),
                    auth_url,
                    Some(token_url),
                )
                .set_redirect_uri(RedirectUrl::new(
                    "http://localhost:3000/oauth/google/callback".to_string(),
                )?)
                .set_revocation_uri(RevocationUrl::new(
                    "https://oauth2.googleapis.com/revoke".to_string(),
                )?)
            },
        }))
        .layer(Extension(Arc::from(jwt::Header::new(
            jwt::Algorithm::HS384,
        ))))
        .layer(Extension(Arc::from(jwt::EncodingKey::from_base64_secret(
            &env::var("HMAC_KEY")?,
        )?)))
        .layer(Extension(Arc::from(jwt::DecodingKey::from_base64_secret(
            &env::var("HMAC_KEY")?,
        )?)))
        .layer(CatchPanicLayer::new())
        .layer(
            livereload.request_predicate(|req: &Request| !req.headers().contains_key("hx-request")),
        )
        .layer(CompressionLayer::new().br(true));

    let listener = TcpListener::bind("0.0.0.0:3000").await?;

    axum::serve(listener, app).await?;

    Ok(())
}
