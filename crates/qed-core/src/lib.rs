use std::{error::Error, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::{uuid, Uuid};

pub const NAMESPACE_QUESTION: Uuid = uuid!("0ef7b0a4-eb18-4108-b99e-cabe7b30b51b");

// This is a wierd case of breaking the ownership model by having a "shared reference" by holding a
// "index" to a object
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Question {
    pub id: Uuid,
    pub document_id: Uuid,
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
pub struct Document {
    pub id: Uuid,
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Auth {
    /// Auth information provided by Google's Oauth2 API using the email, openid, profile scopes
    GoogleOauth(GoogleUser),
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Commentable {
    Question(Question),
    Comment(Comment),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    // TODO: This should be a `Email` type;
    pub email: String,
    // TODO: This should be a `URI` type;
    pub picture: String,
}

impl PartialEq for User {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl User {
    pub fn new(id: Uuid, email: String, picture: String) -> Self {
        Self { id, email, picture }
    }

    pub async fn from_id<E: Error>(
        id: Uuid,
        repo: &impl Repository<Error = E>,
    ) -> Result<User, RepositoryError<E>> {
        repo.get_user_from_id(id).await
    }

    pub async fn from_auth<E: Error>(
        auth: Auth,
        repo: &impl Repository<Error = E>,
    ) -> Result<User, RepositoryError<E>> {
        repo.get_user_from_auth(auth).await
    }

    pub async fn create_comment<E: Error>(
        &self,
        repo: &impl Repository<Error = E>,
        parent: &Commentable,
        content: impl AsRef<str> + Send + Sync,
    ) -> Result<Comment, RepositoryError<E>> {
        repo.add_comment(parent, self, content).await
    }

    pub async fn auth<E: Error>(
        &self,
        repo: &impl Repository<Error = E>,
    ) -> Result<Auth, RepositoryError<E>> {
        repo.get_auth_from_user(self).await
    }
}

/// This structure is based on the return of the Google Oauth2 API
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct GoogleUser {
    pub sub: String,
    pub name: String,
    pub given_name: String,
    pub picture: String,
    pub email: String,
    pub email_verified: bool,
}

#[derive(thiserror::Error, Debug)]
pub enum RepositoryError<E: std::error::Error> {
    #[error("user already exists")]
    UserAlreadyExists(Auth),

    #[error("user doesn't exists")]
    UserNotFound,

    #[error(transparent)]
    Other(#[from] E),
}

/// For the sake of generality, pass a ref to self all the time. Need mutability?
/// [`RefCell`](std::cell::RefCell)  is your friend.

#[async_trait]
pub trait Repository {
    type Error: std::error::Error;

    async fn register_user(&self, auth: Auth) -> Result<User, RepositoryError<Self::Error>>;
    async fn delete_user(&self, user: User) -> Result<(), RepositoryError<Self::Error>>;

    /// Get user handle from auth info. The return type is wrapped in two Result to provide fine
    /// grained resolution of errors. `Result<User, UserError>' might be in the future just a enum
    /// with the variants as the possible cases, but from the function name, returning a error on
    /// anything else then return a `User` struct sounds like a error.
    async fn get_user_from_auth(&self, auth: Auth) -> Result<User, RepositoryError<Self::Error>>;
    async fn get_user_from_id(&self, id: Uuid) -> Result<User, RepositoryError<Self::Error>>;

    async fn get_auth_from_user(&self, user: &User) -> Result<Auth, RepositoryError<Self::Error>>;

    async fn add_question(
        &self,
        document: &Document,
        position: u32,
        tags: Vec<String>,
    ) -> Result<Question, RepositoryError<Self::Error>>;
    async fn get_question(&self, id: Uuid) -> Result<Question, RepositoryError<Self::Error>>;

    async fn add_comment(
        &self,
        parent: &Commentable,
        owner: &User,
        content: impl AsRef<str> + Send + Sync,
    ) -> Result<Comment, RepositoryError<Self::Error>>;
    async fn get_comment_list(
        &self,
        parent: &Commentable,
    ) -> Result<Vec<Comment>, RepositoryError<Self::Error>>;
}
