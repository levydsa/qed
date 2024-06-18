use crate::{infra, App, LibsqlSessionStore, SessionStore, StatusCode};
use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::request::Parts,
    response::{IntoResponse, Response},
};
use std::{ops::Deref, sync::Arc};
use uuid::Uuid;

pub struct User(pub Option<qed_core::User>);

#[derive(thiserror::Error, Debug)]
pub enum UserRejection {
    #[error(transparent)]
    Session(#[from] self::SessionIdRejection),

    #[error(transparent)]
    LibsqlRepository(#[from] qed_core::RepositoryError<infra::libsql::Error>),

    #[error(transparent)]
    Other(#[from] anyhow::Error),

    #[error(transparent)]
    Extension(#[from] axum::extract::rejection::ExtensionRejection),
}

impl IntoResponse for UserRejection {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("{:?}", self)).into_response()
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for User
where
    Arc<App>: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = UserRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app: Arc<App> = FromRef::from_ref(state);
        let repo = app.repository.lock().await;

        let SessionId(id) = SessionId::from_request_parts(parts, state).await.unwrap();
        let session_store = LibsqlSessionStore::new(Arc::clone(&app.db));
        let record = session_store.get(id).await.expect("ensure_session_id should make a valid id :)");

        // TODO: I really want to use `and_then`, but it does not accept a async closure.
        // Function coloring sucks balls.
        Ok(User(match record.user_id {
            Some(id) => qed_core::User::from_id(id, repo.deref()).await.ok(),
            None => None,
        }))
    }
}

#[derive(Clone)]
pub struct SessionId(pub Uuid);

#[derive(thiserror::Error, Debug)]
pub enum SessionIdRejection {
    #[error(transparent)]
    Extension(#[from] axum::extract::rejection::ExtensionRejection),

    #[error(transparent)]
    Uuid(#[from] uuid::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl IntoResponse for SessionIdRejection {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("{:?}", self)).into_response()
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for SessionId
where
    Arc<App>: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = SessionIdRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<SessionId>()
            .cloned()
            .expect("Session must exist"))
    }
}
