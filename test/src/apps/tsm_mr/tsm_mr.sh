#!/bin/sh

set -e

i=0
while [ $i -le 3 ]; do
    echo "Testing RTMR${i}..."
    dd if=/dev/urandom bs=48 count=1 > "/sys/class/misc/tdx_guest/measurements/rtmr${i}:sha384"
    hd "/sys/class/misc/tdx_guest/measurements/rtmr${i}:sha384"
    i=$((i + 1))
done

echo "All RTMR tests completed successfully"
