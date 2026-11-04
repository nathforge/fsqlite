-- Sample database for fsqlite. Build with:
--   sqlite3 examples/sample.sqlite < examples/sample.sql

PRAGMA foreign_keys = ON;

CREATE TABLE users (
  id        INTEGER PRIMARY KEY,
  username  TEXT NOT NULL UNIQUE,
  email     TEXT NOT NULL,
  age       INTEGER,
  profile   TEXT CHECK (json_valid(profile))   -- JSON object
);

CREATE TABLE posts (
  id         INTEGER PRIMARY KEY,
  author_id  INTEGER NOT NULL REFERENCES users(id),
  title      TEXT NOT NULL,
  body       TEXT NOT NULL,
  tags       TEXT CHECK (json_valid(tags)),    -- JSON array
  created_at TEXT NOT NULL
);

-- Composite primary key.
CREATE TABLE post_likes (
  post_id  INTEGER NOT NULL REFERENCES posts(id),
  user_id  INTEGER NOT NULL REFERENCES users(id),
  liked_at TEXT NOT NULL,
  PRIMARY KEY (post_id, user_id)
);

-- Text primary key, JSON values.
CREATE TABLE settings (
  key    TEXT PRIMARY KEY,
  value  TEXT NOT NULL CHECK (json_valid(value))
);

-- Nested JSON payloads.
CREATE TABLE events (
  id       INTEGER PRIMARY KEY,
  type     TEXT NOT NULL,
  payload  TEXT NOT NULL CHECK (json_valid(payload)),
  at       TEXT NOT NULL
);

INSERT INTO users VALUES
 (1, 'alice', 'alice@example.com', 30,
  json('{"display_name":"Alice Liddell","bio":"Curiouser and curiouser.","links":{"web":"https://alice.example","mastodon":"@alice@example.social"},"prefs":{"theme":"dark","notifications":true}}')),
 (2, 'bob', 'bob@example.com', NULL,
  json('{"display_name":"Bob Builder","bio":null,"links":{},"prefs":{"theme":"light","notifications":false}}')),
 (3, 'carol', 'carol@example.com', 41, NULL);

INSERT INTO posts VALUES
 (1, 1, 'Hello, world', 'First post!', json('["intro","meta"]'), '2026-01-05T09:30:00Z'),
 (2, 1, 'Falling down holes', 'Notes on rabbit holes and how to get out of them.', json('["rabbits","travel"]'), '2026-02-11T18:02:00Z'),
 (3, 2, 'Can we fix it?', 'Yes we can.', json('[]'), '2026-03-02T07:45:00Z');

INSERT INTO post_likes VALUES
 (1, 2, '2026-01-05T10:00:00Z'),
 (1, 3, '2026-01-06T12:15:00Z'),
 (2, 3, '2026-02-12T08:00:00Z'),
 (3, 1, '2026-03-02T09:00:00Z');

INSERT INTO settings VALUES
 ('site_name', json('"fsqlite demo"')),
 ('features',  json('{"comments":true,"likes":true,"signup":{"open":false,"invite_only":true}}')),
 ('rate_limits', json('[{"route":"/api/posts","per_minute":60},{"route":"/api/login","per_minute":5}]')),
 ('maintenance/window', json('{"day":"sunday","hour_utc":3}'));

INSERT INTO events VALUES
 (1, 'login',        json('{"user_id":1,"ip":"203.0.113.7","device":{"os":"macOS","browser":"Firefox"}}'), '2026-03-03T08:00:00Z'),
 (2, 'post_created', json('{"user_id":2,"post_id":3,"tags":[],"draft":false}'),                           '2026-03-02T07:45:00Z'),
 (3, 'purchase',     json('{"user_id":3,"items":[{"sku":"A-100","qty":2,"price":9.99},{"sku":"B-200","qty":1,"price":24.5}],"total":44.48}'), '2026-03-04T15:20:00Z');
