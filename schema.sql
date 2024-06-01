

create table users_questions
  ( user_id blob unique not null primary key
  , question_id blob unique not null primary key
  , content text
  , answer int 
  , metadata text
  , is_deleted boolean default false
  , foreign key(user_id) references users(id)
  , foreign key(question_id) references questions(id)
  );

create table questions
  ( id blob unique not null primary key
  , content text
  , answer blob
  , metadata text
  , is_deleted boolean default false
  );

create table users
  ( id blob unique not null primary key autoincrement
  , email text
  , is_deleted boolean default false
  );

create table google_users
  ( sub blob unique not null primary key
  , name text
  , given_name text
  , picture text
  , email text
  , email_verified boolean
  , locale text
  , user_id blob unique not null
  , foreign key(user_id) references users(id)
  );
