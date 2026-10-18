# Release checklist

> Short version: the steps to ship a release of the made-up `shipit` tool, from tagging the commit to checking the packages.

## Steps

1. Tag the commit.
2. Build the packages.

## Checks

Run the tests before tagging.

## History

The release steps changed three times. First the build ran on a laptop.
Then it moved to the build server. Now the build server also signs the
packages and uploads them, so a release takes one command.
