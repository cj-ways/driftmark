# Release checklist

The steps to ship a release.

## Build

```sh
shipit build --release --target linux --target macos --target windows --sign --notarize --upload --channel stable --verbose
```
