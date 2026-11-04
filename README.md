# fsqlite

Mount sqlite DB as a read-only filesystem.

Rows are folders, columns are files. JSON data optionally prettified.

- [Demo](#demo)
- [Install](#install)

## Demo <a name="demo"></a>

```shell
# Show the pre-existing "events" table schema
$ sqlite3 examples/sample.sqlite ".schema events"
CREATE TABLE events (
  id       INTEGER PRIMARY KEY,
  type     TEXT NOT NULL,
  payload  TEXT NOT NULL CHECK (json_valid(payload)),
  at       TEXT NOT NULL
);

# Mount to the _db folder
$ mkdir _db
$ fsqlite examples/sample.sqlite _db --pretty-json
Mounted examples/sample.sqlite to _db. Press Ctrl+C to exit and unmount
```

In another window:

```shell
# What does a row directory contain?
$ find _db/posts/1
_db/posts/1
_db/posts/1/id
_db/posts/1/author_id
_db/posts/1/title
_db/posts/1/body
_db/posts/1/tags
_db/posts/1/created_at

# Search posts
$ rg 'First post!' _db/posts --glob body
_db/posts/1/body

# Extract data from a JSON column
$ jq -r '.user_id' _db/events/*/payload
1
2
3
```

Mounted two databases? You could use `diff -r` to show the differences.

## Install <a name="install"></a>

### Prerequisites

Install a [FUSE implementation](https://en.wikipedia.org/wiki/Filesystem_in_Userspace):

- macOS - [macFUSE](https://macfuse.github.io/)
- Linux - fuse3 should be available in your package manager

### Installation options

Homebrew:

```shell
brew tap nathforge/tap
brew trust nathforge/tap
brew install fsqlite
```

Mise:

```shell
MISE_MINIMUM_RELEASE_AGE=0s mise use github:nathforge/fsqlite
```

Or download & install a [release](https://github.com/nathforge/fsqlite/releases).
