#!/bin/bash

RUNS=3
BURSTS=(130 120 110) 

for BURST in ${BURSTS[@]}
do
    for i in $(seq 1 1 "$RUNS")
    do
        if fab remote --burst $BURST \
            | tee /dev/tty \
            | grep -i "error\|exception\|traceback"
        then
            echo "Failed to complete remote benchmark"
            fab kill
            exit 2
        fi
    fab kill
    sleep 20
    done
    fab stop
    sleep 90
    fab start
    sleep 180
done