use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::{uuid, Uuid};

#[derive(Error, Debug)]
pub enum Error {
    #[error("User already exists")]
    UserAlreadyRegistered,

    #[error("User not registered")]
    UserNotRegistered,

    #[error("User not present")]
    UserAlreadyDeleted,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub const NAMESPACE_QUESTION: Uuid = uuid!("0ef7b0a4-eb18-4108-b99e-cabe7b30b51b");

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Question {
    pub id: Uuid,
    pub parent: Uuid,
    pub position: u32,
    pub tags: Vec<String>,
}

impl PartialEq for Question {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Comment {
    pub id: Uuid,
    pub parent: Arc<Commentable>,
    pub owner: User,
    pub content: String,
}

impl PartialEq for Comment {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub picture: String,
}

impl PartialEq for User {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    pub id: Uuid,
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl User {
    pub async fn create_comment(
        &self,
        repo: &mut impl Repository,
        parent: &Commentable,
        content: impl AsRef<str> + Send + Sync,
    ) -> Result<Comment> {
        repo.add_comment(parent, self, content).await
    }

    pub async fn auth(
        &self,
        repo: &mut impl Repository,
    ) -> Result<Auth> {
        repo.get_auth_from_user(self).await
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct GoogleUser {
    pub sub: String,
    pub name: String,
    pub given_name: String,
    pub picture: String,
    pub email: String,
    pub email_verified: bool,
    pub locale: String,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Auth {
    GoogleOauth(GoogleUser),
}

#[async_trait]
pub trait Repository {
    async fn register_user(&mut self, auth: Auth) -> Result<User>;
    async fn delete_user(&mut self, user: User) -> Result<()>;

    async fn get_user_from_auth(&self, auth: Auth) -> Result<User>;
    async fn get_user_from_id(&self, id: Uuid) -> Result<User>;

    async fn get_auth_from_user(&self, user: &User) -> Result<Auth>;

    async fn add_question(
        &mut self,
        document: &Document,
        position: u32,
        tags: Vec<String>,
    ) -> Result<Question>;
    async fn get_question(&self, id: Uuid) -> Result<Question>;

    async fn add_comment(
        &mut self,
        parent: &Commentable,
        owner: &User,
        content: impl AsRef<str> + Send + Sync,
    ) -> Result<Comment>;
    async fn get_comment_list(&self, parent: &Commentable) -> Result<Vec<Comment>>;
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Commentable {
    Question(Question),
    Comment(Comment),
}
