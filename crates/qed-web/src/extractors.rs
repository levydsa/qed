
use std::sync::Arc;
use axum::{async_trait, extract::{FromRef, FromRequestParts}, http::request::Parts, response::{IntoResponse, Response}, Extension};
use axum_extra::extract::CookieJar;
use jsonwebtoken as jwt;
use qed_core::Repository;
use crate::{App, Claims, StatusCode};

pub struct User(pub Option<qed_core::User>);

#[derive(thiserror::Error, Debug)]
pub enum UserRejection {
    #[error(transparent)]
    Jwt(#[from] jwt::errors::Error),

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
        use axum::RequestPartsExt;

        let app: Arc<App> = FromRef::from_ref(state);

        let mut repo = app.repository.lock().await;

        let jar: CookieJar = parts
            .extract()
            .await
            .expect("`CookieJar` rejection is `Infaliable`");
        let Extension(dec_key) = parts.extract::<Extension<Arc<jwt::DecodingKey>>>().await?;
        let Extension(header) = parts.extract::<Extension<Arc<jwt::Header>>>().await?;

        Ok(User(match jar.get("auth") {
            Some(auth) => {
                let mut validation = jwt::Validation::new(header.alg);
                let token: jwt::TokenData<Claims> =
                    jwt::decode(auth.value(), &dec_key, &validation)?;

                repo.get_user_from_id(token.claims.sub).await.ok()
            }
            None => None,
        }))
    }
}
