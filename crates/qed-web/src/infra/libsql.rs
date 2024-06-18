use crate::{async_trait, LibsqlValueRefExt};
use ::libsql::params;
use libsql::named_params;
use std::{process::id, sync::Arc};
use uuid::{Timestamp, Uuid};

pub struct LibsqlRepository {
    db: Arc<libsql::Database>,
}

impl LibsqlRepository {
    pub async fn new(db: Arc<libsql::Database>) -> crate::Result<Self> {
        Ok(Self { db })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Libsql(#[from] libsql::Error),

    #[error(transparent)]
    Uuid(#[from] uuid::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

use qed_core::{Auth, Comment, Commentable, Document, Question, RepositoryError, User};

#[async_trait]
impl qed_core::Repository for LibsqlRepository {
    type Error = Error;

    async fn register_user(&self, auth: Auth) -> Result<User, RepositoryError<Self::Error>> {
        let conn = self.db.connect().map_err(|err| Error::from(err))?;

        match auth {
            qed_core::Auth::GoogleOauth(google_user) => {
                let mut rows = conn
                    .query(
                        "SELECT count(*) FROM google_users WHERE sub = ?1",
                        params![google_user.sub.to_value()],
                    )
                    .await
                    .map_err(|err| Error::from(err))?;

                let row = rows.next().await.map_err(|err| Error::from(err))?.unwrap();
                if row.get::<i64>(0).map_err(|err| Error::from(err))? != 0 {
                    return Err(qed_core::RepositoryError::UserAlreadyExists(
                        qed_core::Auth::GoogleOauth(google_user),
                    ));
                };

                let user = User::new(
                    Uuid::now_v7(),
                    google_user.email.clone(),
                    google_user.picture.clone(),
                );

                conn.execute(
                    r#"
                    INSERT OR IGNORE INTO google_users
                        (  sub,  name,  given_name,  picture,  email,  email_verified )
                    VALUES
                        ( :sub, :name, :given_name, :picture, :email, :email_verified )
                    "#,
                    named_params! {
                        ":sub": google_user.sub.to_value(),
                        ":name": google_user.name.to_value(),
                        ":given_name": google_user.given_name.to_value(),
                        ":picture": google_user.picture.to_value(),
                        ":email": google_user.email.to_value(),
                        ":email_verified": google_user.email_verified,
                    },
                )
                .await
                .map_err(|err| Error::from(err))?;

                conn.execute(
                    r#"
                    INSERT OR IGNORE INTO users
                        (  id,  email,  picture )
                    VALUES
                        ( :id, :email, :picture )
                    "#,
                    named_params! {
                        ":id": user.id.to_value(),
                        ":email": user.email.to_value(),
                        ":picture": user.picture.to_value(),
                    },
                )
                .await
                .map_err(|err| Error::from(err))?;

                conn.execute(
                    r#"
                    INSERT OR IGNORE INTO google_users_users
                        (  user_id,  google_user_sub )
                    VALUES
                        ( :user_id, :google_user_sub )
                    "#,
                    named_params! {
                        ":user_id": user.id.to_value(),
                        ":google_user_sub": google_user.sub.to_value(),

                    },
                )
                .await
                .map_err(|err| Error::from(err))?;

                Ok(user)
            }
        }
    }

    async fn delete_user(&self, user: User) -> Result<(), RepositoryError<Self::Error>> {
        todo!()
    }

    async fn get_user_from_auth(&self, auth: Auth) -> Result<User, RepositoryError<Self::Error>> {
        let conn = self.db.connect().map_err(|err| Error::from(err))?;

        match auth {
            qed_core::Auth::GoogleOauth(google_user) => {
                let mut rows = conn
                    .query(
                        r#"
                        SELECT id, email, picture
                        FROM google_users_users gu
                        INNER JOIN users u ON u.id = gu.user_id
                        WHERE google_user_sub = ?1
                        "#,
                        params![google_user.sub.to_value()],
                    )
                    .await
                    .map_err(|err| Error::from(err))?;

                if let Some(row) = rows.next().await.map_err(|err| Error::from(err))? {
                    let id = row
                        .get_value(0)
                        .map_err(|err| Error::from(err))?
                        .as_blob()
                        .map(|v| Uuid::from_slice(v.as_slice()))
                        .expect("field should be a uuid blob")
                        .map_err(|err| Error::from(err))?;
                    let email = row.get_str(1).map_err(|err| Error::from(err))?.to_owned();
                    let picture = row.get_str(2).map_err(|err| Error::from(err))?.to_owned();

                    Ok(User::new(id, email, picture))
                } else {
                    Err(RepositoryError::UserNotFound)
                }
            }
        }
    }

    async fn get_user_from_id(&self, id: Uuid) -> Result<User, RepositoryError<Self::Error>> {
        let conn = self.db.connect().map_err(|err| Error::from(err))?;

        let mut rows = conn
            .query(
                r#"
                SELECT id, email, picture
                FROM users
                WHERE id = ?1
                "#,
                params![id.to_value()],
            )
            .await
            .map_err(|err| Error::from(err))?;

        if let Some(row) = rows.next().await.map_err(|err| Error::from(err))? {
            let id = row
                .get_value(0)
                .map_err(|err| Error::from(err))?
                .as_blob()
                .map(|v| Uuid::from_slice(v.as_slice()))
                .expect("field should be a uuid blob")
                .map_err(|err| Error::from(err))?;
            let email = row.get_str(1).map_err(|err| Error::from(err))?.to_owned();
            let picture = row.get_str(2).map_err(|err| Error::from(err))?.to_owned();

            Ok(User::new(id, email, picture))
        } else {
            Err(RepositoryError::UserNotFound)
        }
    }

    async fn get_auth_from_user(&self, user: &User) -> Result<Auth, RepositoryError<Self::Error>> {
        todo!()
    }

    async fn add_question(
        &self,
        document: &Document,
        position: u32,
        tags: Vec<String>,
    ) -> Result<Question, RepositoryError<Self::Error>> {
        let conn = self.db.connect().map_err(|err| Error::from(err))?;
        let doc_id = document.id;

        let q = qed_core::Question {
            id: Uuid::new_v5(
                &qed_core::NAMESPACE_QUESTION,
                format!("{doc_id}{position}").as_ref(),
            ),
            document_id: doc_id,
            position,
            tags,
        };

        conn.execute(
            r#"
            INSERT OR FAIL INTO questions
                (  id,  document_id,  position,  tags )
            VALUES
                ( :id, :document_id, :position, :tags )
            "#,
            named_params! {
                ":id": q.id.to_value(),
                ":document_id": q.document_id.to_value(),
                ":position": q.position,
                ":tags": serde_json::to_string(&q.tags).map_err(|err| Error::from(err))?,
            },
        );

        Ok(q)
    }

    async fn get_question(&self, id: Uuid) -> Result<Question, RepositoryError<Self::Error>> {
        todo!()
    }

    async fn add_comment(
        &self,
        parent: &Commentable,
        owner: &User,
        content: impl AsRef<str> + Send + Sync,
    ) -> Result<Comment, RepositoryError<Self::Error>> {
        todo!()
    }

    async fn get_comment_list(
        &self,
        parent: &Commentable,
    ) -> Result<Vec<Comment>, RepositoryError<Self::Error>> {
        todo!()
    }
}
