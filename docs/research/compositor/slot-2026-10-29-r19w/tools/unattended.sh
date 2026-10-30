#!/bin/sh
# Round 19W: the unattended steps in order, each logged; a family sign (/tmp/r19w-gate/FAMILY) stops the chain.
cd /tmp/r19w-build
L=/tmp/r19w-gate/unattended.log
echo "$(date +%T) unattended start" >> $L
python3 chain19w.py "$@" >> $L 2>&1
echo "$(date +%T) chain19w exit $?" >> $L
