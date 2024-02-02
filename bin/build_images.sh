#!/bin/sh
# Build base image
podman build --tag="mls_build:v0.1.0" --file="./container/base-Containerfile" "."
# Build binaries
podman run --volume=./:/mls:ro  --volume=./release:/mls/release --tmpfs /mls/target "mls_build:v0.1.0"
# Build the MLS Proxy image
#podman build --tag="mls_proxy:v0.1.0" --file="./container/proxy-Containerfile" "."
# Build the MLS LabelDB image
#podman build --tag="mls_labeldb:v0.1.0" --file="./container/labeldb-Containerfile" "."
