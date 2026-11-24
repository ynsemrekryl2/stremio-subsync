# syntax=docker/dockerfile:1
# The SubSync server as a container image (see compose.yaml).
#
#   docker build -t stremio-subsync .
#
# builds it from source for this machine. The release workflow passes
# BINARY=release instead and copies the binaries it already built from
# dist/, so one push makes the x86_64 and ARM64 image with no emulated
# compile.
ARG BINARY=source

FROM rust:1.98-slim-bookworm AS source
WORKDIR /src
COPY . .
RUN cargo build --release --locked \
	&& mkdir -p /out/data \
	&& cp target/release/stremio-subsync /out/

# Runs on the build machine whatever the target: it only picks a file.
FROM --platform=$BUILDPLATFORM debian:bookworm-slim AS release
ARG TARGETARCH
COPY dist/ /dist/
RUN case "$TARGETARCH" in \
		amd64) arch=x86_64 ;; \
		arm64) arch=aarch64 ;; \
		*) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
	esac \
	&& mkdir -p /out/data \
	&& install -m 755 "/dist/stremio-subsync-$arch-linux" /out/stremio-subsync

FROM ${BINARY} AS binary

# glibc, libgcc and the CA roots the TLS client trusts; no shell. Runs as
# the image's nonroot user (65532).
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=binary /out/stremio-subsync /usr/local/bin/stremio-subsync
# A named volume over /data starts with this owner.
COPY --from=binary --chown=65532:65532 /out/data /data
ENV SUBSYNC_BIND=0.0.0.0:7001 \
	SUBSYNC_CACHE_DIR=/data/cache \
	SUBSYNC_SETTINGS_FILE=/data/settings.json \
	SUBSYNC_SECRETS_FILE=/data/secrets.json
VOLUME /data
EXPOSE 7001
# The server shuts down gracefully on Ctrl-C only.
STOPSIGNAL SIGINT
ENTRYPOINT ["/usr/local/bin/stremio-subsync"]
