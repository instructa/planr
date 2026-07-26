ARG BUILD_IMAGE=rust:1.90.0-alpine3.21@sha256:3757b14ddcc2057eb91a074dcdd0913bed839b22444bd2229a49eea910ed8736
FROM ${BUILD_IMAGE}

ARG MUSL_VERSION
ARG MUSL_SHA256
ARG MUSL_DEV_SHA256

COPY musl.apk /tmp/musl.apk
COPY musl-dev.apk /tmp/musl-dev.apk

RUN printf '%s  %s\n%s  %s\n' \
      "$MUSL_SHA256" /tmp/musl.apk \
      "$MUSL_DEV_SHA256" /tmp/musl-dev.apk \
      | sha256sum -c - \
    && apk verify /tmp/musl.apk /tmp/musl-dev.apk \
    && apk --no-network --repositories-file /dev/null add --no-cache /tmp/musl.apk /tmp/musl-dev.apk \
    && apk list --installed musl | grep -qx "musl-${MUSL_VERSION} .*" \
    && apk list --installed musl-dev | grep -qx "musl-dev-${MUSL_VERSION} .*" \
    && test -f /usr/include/stdio.h \
    && rm -f /tmp/musl.apk /tmp/musl-dev.apk
