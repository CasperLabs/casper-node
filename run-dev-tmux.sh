#!/usr/bin/env bash

set -o errexit
set -o nounset
set -o pipefail

tmux_new_window() {
    local SESSION=${1}
    local ID=${2}
    local CMD=${3}
    local NAME="${SESSION}-${ID}"

    tmux new-window -t "${SESSION}:${ID}" -n "${NAME}"
    tmux send-keys -t "${NAME}" "${CMD}" C-m
}

build_system_contracts() {
    local CMD=(
        "make -s"
        "build-contracts-rs"
        "CARGO_FLAGS=--quiet"
    )

    echo "Building system contracts..."
    ${CMD[*]}
}

build_node() {
    local CMD=(
        "cargo build"
        "--quiet"
        "--manifest-path=node/Cargo.toml"
    )

    echo "Building node..."
    ${CMD[*]}
}

generate_timestamp() {
    local DELAY=${1}

    local SCRIPT=(
        "from datetime import datetime, timedelta;"
        "print((datetime.utcnow() + timedelta(seconds=${DELAY})).isoformat('T') + 'Z')"
    )

    python3 -c "${SCRIPT[*]}"
}

generate_chainspec() {
    local BASEDIR=${1}
    local TIMESTAMP=${2}
    local SOURCE="${BASEDIR}/resources/local/chainspec.toml.in"
    local TARGET="${BASEDIR}/resources/local/chainspec.toml"

    export BASEDIR
    export TIMESTAMP

    echo "Generating chainspec..."
    envsubst < ${SOURCE} > ${TARGET}
}

run_node() {
    local EXECUTABLE=${1}
    local SESSION=${2}
    local ID=${3}
    local CONFIG_DIR=${4}
    local DATA_DIR=${5}
    local CONFIG_TOML_PATH="${CONFIG_DIR}/config.toml"
    local SECRET_KEY_PATH="${CONFIG_DIR}/secret_keys/node-${ID}.pem"
    local STORAGE_DIR="${DATA_DIR}/node-${ID}-storage"

    local CMD=(
        "${EXECUTABLE}"
        "validator"
        "${CONFIG_TOML_PATH}"
        "-C consensus.secret_key_path=${SECRET_KEY_PATH}"
        "-C storage.path=${STORAGE_DIR}"
        "-C rpc_server.address='0.0.0.0:50101'"
    )

    if [[ ${ID} != 1 ]]; then
        CMD+=("-C network.bind_address='0.0.0.0:0'")
        CMD+=("-C rpc_server.address='0.0.0.0:0'")
        CMD+=("-C rest_server.address='0.0.0.0:0'")
        CMD+=("-C event_stream_server.address='0.0.0.0:0'")
    fi

    CMD+=("1> >(tee ${DATA_DIR}/node-${ID}.log) 2> >(tee ${DATA_DIR}/node-${ID}.log.stderr)")

    mkdir -p "${STORAGE_DIR}"
    tmux_new_window "${SESSION}" "${ID}" "${CMD[*]}"
    echo "Booting node ${ID}..."
}

check_for_bootstrap () {
    local BOOTSTRAP_PORT=34553

    while ! (: </dev/tcp/0.0.0.0/${BOOTSTRAP_PORT}) &>/dev/null; do
        sleep 1
    done
}

main() {
    local DELAY=${1:-40}
    local SESSION="${SESSION:-local}"
    local TMPDIR="${TMPDIR:-$(mktemp -d)}"
    local BASEDIR="$(readlink -f $(dirname ${0}))"
    local EXECUTABLE="${BASEDIR}/target/debug/casper-node"
    local CONFIG_DIR="${BASEDIR}/resources/local"
    local TIMESTAMP="$(generate_timestamp ${DELAY})"
    local RUST_LOG="${RUST_LOG:-debug}"

    export TMPDIR
    export RUST_LOG

    build_system_contracts

    build_node

    generate_chainspec ${BASEDIR} ${TIMESTAMP}

    tmux new-session -d -s ${SESSION}

    local ID=1
    run_node ${EXECUTABLE} ${SESSION} ${ID} ${CONFIG_DIR} ${TMPDIR}

    for ID in {2..5}; do
        check_for_bootstrap
        run_node ${EXECUTABLE} ${SESSION} ${ID} ${CONFIG_DIR} ${TMPDIR}
    done

    echo
    echo "DELAY     : ${DELAY}"
    echo "TMPDIR    : ${TMPDIR}"
    echo "TIMESTAMP : ${TIMESTAMP}"
    echo "RUST_LOG  : ${RUST_LOG}"
    echo
    echo "To view: "
    echo "    tmux attach -t ${SESSION}"
    echo
    echo "To kill: "
    echo "    tmux kill-session -t ${SESSION}"
    echo
}

main $@
