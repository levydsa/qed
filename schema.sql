
create table sessions
  ( id blob unique not null primary key
  , data text
  );

create table users_questions
  ( user_id blob unique not null
  , question_id blob unique not null
  , answer blob not null
  , foreign key(user_id) references users(id)
  , foreign key(question_id) references questions(id)
  );

create table comments
  ( id blob unique not null primary key
  , user_id blob unique not null
  , parent_id blob unique not null
  , content text not null
  , foreign key(user_id) references users(id)
  );

create table questions
  ( id blob unique not null primary key
  , document_id blob unique not null
  , position int
  , tags text
  );

create table google_users_users
  ( user_id blob unique not null
  , google_user_sub blob unique not null
  , foreign key(user_id) references users(id)
  , foreign key(google_user_sub) references google_users(sub)
  );

create table users
  ( id blob unique not null primary key
  , email text
  , picture text
  );

create table google_users
  ( sub blob unique not null primary key
  , name text
  , given_name text
  , picture text
  , email text
  , email_verified boolean
  );
