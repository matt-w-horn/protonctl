#!/bin/sh
# Run a command against a throwaway GNOME Keyring, unlocked with the
# password "test", in the D-Bus session this script runs in, and stop the
# keyring when the command ends. Start it with dbus-run-session, and point
# XDG_DATA_HOME and XDG_RUNTIME_DIR at a folder of the caller's own, so the
# user's own keyring is never touched and nothing is left in ~/.cache.
#
#   dbus-run-session -- scripts/with-keyring.sh COMMAND [ARG...]
set -u

# In the foreground, so this script can stop it: started on its own, the
# daemon leaves the session and outlives it.
printf test | gnome-keyring-daemon --foreground --unlock --components=secrets >/dev/null &
daemon=$!
trap 'kill "$daemon" 2>/dev/null; wait "$daemon" 2>/dev/null' EXIT

# The keyring owns its name a moment after it starts. A call before that
# would have the bus start a second keyring, which nothing stops.
tries=0
until dbus-send --session --print-reply --dest=org.freedesktop.DBus / \
    org.freedesktop.DBus.NameHasOwner string:org.freedesktop.secrets 2>/dev/null |
    grep -q "boolean true"; do
    tries=$((tries + 1))
    if [ "$tries" -gt 50 ]; then
        echo "with-keyring.sh: the throwaway keyring did not start" >&2
        exit 1
    fi
    sleep 0.1
done

"$@"
status=$?
exit "$status"
