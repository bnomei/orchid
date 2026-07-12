# syntax=docker/dockerfile:1

ARG ORCHID_VERSION=0.7.1
ARG ORCHID_REPOSITORY=bnomei/orchid
ARG ORCHID_RUNTIME_IMAGE=gcr.io/distroless/static-debian13:nonroot

FROM --platform=$BUILDPLATFORM alpine:3.22 AS fetch
ARG ORCHID_VERSION
ARG ORCHID_REPOSITORY
ARG TARGETARCH

RUN apk add --no-cache ca-certificates curl

RUN set -eux; \
  case "$TARGETARCH" in \
    amd64) target=x86_64-unknown-linux-musl ;; \
    arm64) target=aarch64-unknown-linux-musl ;; \
    *) echo "unsupported Docker target architecture: $TARGETARCH" >&2; exit 1 ;; \
  esac; \
  version="${ORCHID_VERSION#v}"; \
  tag="v${version}"; \
  archive="orchid-${version}-${target}.tar.gz"; \
  url="https://github.com/${ORCHID_REPOSITORY}/releases/download/${tag}/${archive}"; \
  curl -fsSL -o "/tmp/${archive}" "$url"; \
  curl -fsSL -o "/tmp/${archive}.sha256" "${url}.sha256"; \
  cd /tmp; \
  sha256sum -c "${archive}.sha256"; \
  tar -xzf "$archive"; \
  test -f orchid; \
  chmod 755 orchid; \
  mkdir -p /tmp/orchid-workspace

FROM ${ORCHID_RUNTIME_IMAGE}
ARG ORCHID_VERSION

ENV HOME=/workspace

LABEL org.opencontainers.image.title="Orchid"
LABEL org.opencontainers.image.description="Scoped coding-agent orchestration CLI"
LABEL org.opencontainers.image.source="https://github.com/bnomei/orchid"
LABEL org.opencontainers.image.licenses="MIT"
LABEL org.opencontainers.image.version="${ORCHID_VERSION}"

COPY --from=fetch --chown=65532:65532 /tmp/orchid /usr/local/bin/orchid
COPY --from=fetch --chown=65532:65532 /tmp/orchid-workspace /workspace

WORKDIR /workspace
USER 65532:65532

ENTRYPOINT ["/usr/local/bin/orchid"]
