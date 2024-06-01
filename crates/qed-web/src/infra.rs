use uuid::Uuid;

use crate::async_trait;

pub struct MemoryRepository {
    users: Vec<qed_core::User>,
    comments: Vec<qed_core::Comment>,
    question: Vec<qed_core::Question>,
    google_users: Vec<(qed_core::GoogleUser, qed_core::User)>,
}

impl MemoryRepository {
    pub fn new() -> Self {
        Self {
            users: Vec::new(),
            comments: Vec::new(),
            question: Vec::new(),
            google_users: Vec::new(),
        }
    }
}

#[async_trait]
impl qed_core::Repository for MemoryRepository {
    async fn register_user(&mut self, auth: qed_core::Auth) -> qed_core::Result<qed_core::User> {
        match auth {
            qed_core::Auth::GoogleOauth(google_user) => {
                if self.google_users.iter().any(|(a, _)| a == &google_user) {
                    return Err(qed_core::Error::UserAlreadyRegistered);
                }

                let user = qed_core::User {
                    id: Uuid::new_v4(),
                    picture: google_user.picture.clone(),
                };

                self.google_users.push((google_user, user.clone()));
                self.users.push(user.clone());

                Ok(user)
            }
        }
    }

    async fn delete_user(&mut self, user: qed_core::User) -> qed_core::Result<()> {
        self.users
            .remove(self.users.iter().position(|u| u == &user).unwrap());
        self.google_users.remove(
            self.google_users
                .iter()
                .position(|(_, u)| u == &user)
                .unwrap(),
        );

        Ok(())
    }

    async fn get_user_from_auth(&self, auth: qed_core::Auth) -> qed_core::Result<qed_core::User> {
        match auth {
            qed_core::Auth::GoogleOauth(google_user) => {
                if let Some((_, user)) = self.google_users.iter().find(|(a, _)| *a == google_user) {
                    Ok(user.clone())
                } else {
                    Err(qed_core::Error::UserNotRegistered)
                }
            }
        }
    }

    async fn get_user_from_id(&self, id: Uuid) -> qed_core::Result<qed_core::User> {
        self.users
            .iter()
            .find(|user| user.id == id)
            .cloned()
            .ok_or(qed_core::Error::UserNotRegistered)
    }

    async fn get_auth_from_user(&self, user: &qed_core::User) -> qed_core::Result<qed_core::Auth> {
        self.google_users
            .iter()
            .find(|(_, u)| *u == *user)
            .cloned()
            .map(|(google_user, _)| qed_core::Auth::GoogleOauth(google_user))
            .ok_or(qed_core::Error::UserNotRegistered)
    }

    async fn add_question(
        &mut self,
        document: &qed_core::Document,
        position: u32,
        tags: Vec<String>,
    ) -> qed_core::Result<qed_core::Question> {
        let doc_id = document.id;

        let q = qed_core::Question {
            id: Uuid::new_v5(
                &qed_core::NAMESPACE_QUESTION,
                format!("{doc_id}{position}").as_ref(),
            ),
            parent: doc_id,
            position,
            tags,
        };

        self.question.push(q.clone());

        Ok(q)
    }

    async fn get_question(&self, id: Uuid) -> qed_core::Result<qed_core::Question> {
        todo!()
    }

    async fn add_comment(
        &mut self,
        parent: &qed_core::Commentable,
        owner: &qed_core::User,
        content: impl AsRef<str> + Send + Sync,
    ) -> qed_core::Result<qed_core::Comment> {
        todo!()
    }

    async fn get_comment_list(
        &self,
        parent: &qed_core::Commentable,
    ) -> qed_core::Result<Vec<qed_core::Comment>> {
        use qed_core::Commentable as C;
        use qed_core::{Comment, Question};

        Ok(self
            .comments
            .iter()
            .filter(|comment| *comment.parent == *parent)
            .cloned()
            .collect::<Vec<Comment>>())
    }
}
