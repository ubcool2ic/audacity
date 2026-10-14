#!/bin/sh
cd "$(dirname "$0")" && g++ -O2 -o spec_ref spec_ref.cpp /usr/lib/audacity/lib-fft.so -Wl,-rpath,/usr/lib/audacity
