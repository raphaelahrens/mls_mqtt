#!/bin/sh

printf 'Subscribing to %s ...\n' "$1"
sleep "$2"
echo 'now\n'
mosquitto_sub -h localhost -p 21883 -q 2 -t "$1" -F 'topic:%t len:%l QoS: %q ID: %m \n\n |%p|\n\n%x'
