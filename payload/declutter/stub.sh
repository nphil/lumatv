#!/bin/sh
# Bound by tools/declutter.sh over the executable of an on-demand LG service that
# is switched off. ls-hubd runs it in the service's place, it exits at once, and
# the caller gets an error instead of a daemon.
exit 0
