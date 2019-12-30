#!/bin/bash

# This script is called by the Travis CI pipeline when a new tag has been created.
# wasm-pack creates a ready-to-publish NPM package into a directory called pkg.

cd "$TRAVIS_BUILD_DIR/pkg"
npm config set "//registry.npmjs.org/:_authToken" "$NPM_API_KEY"
npm publish
