# Release checklist

> Short version: the steps to ship a release of the made-up `shipit` tool, from tagging the commit to checking the packages.

## Steps

1. Tag the commit.
2. Build the packages.

## Checks

Run the tests before tagging.

```sh
shipit build --release \
  --target linux
```

The build prints one long line when it is done:

```text
shipit: built the linux, macos and windows packages for the stable channel, signed and notarized them, then uploaded all three
```

And the status command:

```console
$ shipit status --channel stable --verbose --show-signatures --show-checksums --show-upload-times --format table --no-color
```
