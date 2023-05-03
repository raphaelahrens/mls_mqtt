
printf 'Subscribing to %s ...\n' "$1"
sleep 20
echo 'now\n'
mosquitto_sub -h localhost -p 21883 -q 2 -t "$1" -F 'topic:%t len:%l QoS: %q ID: %m \n |%p|\n %x'
