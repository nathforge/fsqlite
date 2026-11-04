# fsqlite

Mount sqlite DB as a read-only filesystem.

Rows are folders, columns are files. JSON data optionally prettified.

 * [Demo](#demo)
 * [Install](#install)

## Demo <a name="demo"></a>

```shell
$ mkdir _db
$ fsqlite examples/sample.sqlite _db --pretty-json
```

In another window:

```shell
$ ls _db/events/3
at	id	payload	type

$ jq '.items[0]' _db/events/3/payload
{
  "sku": "A-100",
  "qty": 2,
  "price": 9.99
}
```

## Install <a name="install"></a>

Install a [FUSE implementaion](https://en.wikipedia.org/wiki/Filesystem_in_Userspace):
 * macOS - [macFUSE](https://macfuse.github.io/)
 * Linux - fuse3 should be available in your package manager

TODO (build, homebrew?)
