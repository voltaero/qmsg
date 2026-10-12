#!/bin/sh
# Start usbmuxd inside the container (talks to the iPhone over the passed-through USB bus)
if [ ! -S /var/run/usbmuxd ]; then
    usbmuxd -U root -f >/tmp/usbmuxd.log 2>&1 &
    for _ in 1 2 3 4 5 6 7 8 9 10; do [ -S /var/run/usbmuxd ] && break; sleep 0.3; done
    # Give usbmuxd a moment to enumerate the iPhone
    for _ in 1 2 3 4 5 6 7 8 9 10; do [ -n "$(idevice_id -l 2>/dev/null)" ] && break; sleep 0.3; done
fi
exec "$@"
