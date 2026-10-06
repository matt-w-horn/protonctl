#!/bin/sh
# The hermetic IMAP test (RFC section 7, T11): protonctl's mail reads against
# a real IMAP server, Dovecot, in a throwaway podman container on a free port
# of 127.0.0.1. The test seeds the mailbox itself, then reads it as Bridge's.
# The container is removed when this script ends.
set -eu
cd "$(dirname "$0")/.."

image=docker.io/dovecot/dovecot:2.3.21@sha256:1c18c756f20d03867077a1b509a6e2e3008ab1eafa56377b6f2eca12dc1ba581
name="protonctl-dovecot-$$"
# Set before the container starts; dash runs an EXIT trap on a signal only
# when the signal's own trap exits.
trap 'podman rm --force --time 0 "$name" >/dev/null 2>&1' EXIT
trap 'exit 130' INT TERM
# `z` lets the container read the file where SELinux is enforcing.
podman run --detach --rm --name "$name" --publish 127.0.0.1::993 \
    --volume "$PWD/tests/fixtures/dovecot.conf:/etc/dovecot/dovecot.conf:ro,z" \
    "$image" >/dev/null
port=$(podman port "$name" 993/tcp | sed 's/.*://')

PROTONCTL_DOVECOT_PORT="$port" cargo test --locked --bin protonctl \
    mail::read::tests::every_operation_against_dovecot -- --ignored --exact
