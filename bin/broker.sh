#/bin/sh

run_broker() {
    podman run \
        --mount type=bind,target=/etc/rabbitmq/enabled_plugins,source=/home/ahrens/projects/work/knowledge/mls/mls_mqtt/data/enabled_plugins \
        -p "$1":1883 \
        --rm \
        -i \
        --hostname edge-rabbit \
        --name "edge-rabbit_$1" \
        rabbitmq
}

main() {
    if [ $# -ne 1 ]; then 
        exit 1
    fi
    run_broker "$1"
}

main "$@"

