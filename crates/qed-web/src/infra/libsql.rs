use crate::{async_trait, LibsqlValueExt};
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

use qed_core::{Auth, Comment, Commentable, Document, Question, User, UserError};

#[async_trait]
impl qed_core::Repository for LibsqlRepository {
    type Error = Error;

    async fn register_user(&mut self, auth: Auth) -> Result<Result<User, UserError>, Self::Error> {
        let conn = self.db.connect()?;

        Ok(match auth {
            qed_core::Auth::GoogleOauth(google_user) => 'user: {
                let user = User::new(
                    Uuid::now_v7(),
                    google_user.email.clone(),
                    google_user.picture.clone(),
                );

                let mut rows = conn
                    .query(
                        "SELECT count(*) FROM google_users WHERE sub = ?1",
                        params![google_user.sub.to_value()],
                    )
                    .await?;

                let row = rows.next().await?.unwrap();
                if row.get::<i64>(0)? != 0 {
                    break 'user Err(qed_core::UserError::UserAlreadyExists);
                };

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
                .await?;

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
                .await?;

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
                .unwrap();

                Ok(user)
            }
        })
    }

    async fn delete_user(&mut self, user: User) -> Result<(), Self::Error> {
        todo!()
    }

    async fn get_user_from_auth(&self, auth: Auth) -> Result<Result<User, UserError>, Self::Error> {
        let conn = self.db.connect()?;

        Ok(match auth {
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
                    .await?;

                if let Some(row) = rows.next().await? {
                    let id = row
                        .get_value(0)?
                        .as_blob()
                        .map(|v| Uuid::from_slice(v.as_slice()))
                        .expect("field should be a uuid blob")?;
                    let email = row.get_str(1)?.to_owned();
                    let picture = row.get_str(2)?.to_owned();

                    Ok(User::new(id, email, picture))
                } else {
                    Err(UserError::UserNotFound)
                }
            }
        })
    }

    async fn get_user_from_id(&self, id: Uuid) -> Result<Result<User, UserError>, Self::Error> {
        let conn = self.db.connect()?;

        Ok({
            let mut rows = conn
                .query(
                    r#"
                    SELECT id, email, picture
                    FROM users
                    WHERE id = ?1
                    "#,
                    params![id.to_value()],
                )
                .await?;

            if let Some(row) = rows.next().await? {
                let id = row
                    .get_value(0)?
                    .as_blob()
                    .map(|v| Uuid::from_slice(v.as_slice()))
                    .expect("field should be a uuid blob")?;
                let email = row.get_str(1)?.to_owned();
                let picture = row.get_str(2)?.to_owned();

                Ok(User::new(id, email, picture))
            } else {
                Err(UserError::UserNotFound)
            }
        })
    }

    async fn get_auth_from_user(&self, user: &User) -> Result<Auth, Self::Error> {
        todo!()
    }

    async fn add_question(
        &mut self,
        document: &Document,
        position: u32,
        tags: Vec<String>,
    ) -> Result<Question, Self::Error> {
        let conn = self.db.connect()?;
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
                ":tags": serde_json::to_string(&q.tags)?,
            },
        );

        Ok(q)
    }

    async fn get_question(&self, id: Uuid) -> Result<Question, Self::Error> {
        todo!()
    }

    async fn add_comment(
        &mut self,
        parent: &Commentable,
        owner: &User,
        content: impl AsRef<str> + Send + Sync,
    ) -> Result<Comment, Self::Error> {
        todo!()
    }

    async fn get_comment_list(&self, parent: &Commentable) -> Result<Vec<Comment>, Self::Error> {
        todo!()
    }
}
