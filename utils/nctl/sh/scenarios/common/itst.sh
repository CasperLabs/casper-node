#!/usr/bin/env bash

source "$NCTL"/sh/utils/main.sh
source "$NCTL"/sh/views/utils.sh
source "$NCTL"/sh/node/svc_"$NCTL_DAEMON_TYPE".sh

function log_step() {
    local COMMENT=${1}
    log "------------------------------------------------------------"
    log "STEP $STEP: $COMMENT"
    STEP=$((STEP + 1))
}

function do_await_genesis_era_to_complete() {
    log_step "awaiting genesis era to complete"
    while [ "$(get_chain_era)" != "1" ]; do
        sleep 1.0
    done
}

function do_read_lfb_hash() {
    local NODE_ID=${1}
    LFB_HASH=$(render_last_finalized_block_hash "$NODE_ID" | cut -f2 -d= | cut -f2 -d ' ')
    echo "$LFB_HASH"
}

function do_start_node() {
    local NODE_ID=${1}
    local LFB_HASH=${2}
    log_step "starting node-$NODE_ID. Syncing from hash=${LFB_HASH}"
    do_node_start "$NODE_ID" "$LFB_HASH"
    sleep 1
    if [ "$(do_node_status ${NODE_ID} | awk '{ print $2 }')" != "RUNNING" ]; then
        log "ERROR: node-${NODE_ID} is not running"
	exit 1
    else
        log "node-${NODE_ID} is running"
    fi
}

function do_stop_node() {
    local NODE_ID=${1}
    log_step "stopping node-$NODE_ID."
    do_node_stop "$NODE_ID"
    sleep 1
    if [ "$(do_node_status ${NODE_ID} | awk '{ print $2 }')" = "RUNNING" ]; then
        log "ERROR: node-${NODE_ID} is still running"
	exit 1
    else
        log "node-${NODE_ID} was shutdown in era: $(check_current_era)"
    fi
}

function check_network_sync() {
    local WAIT_TIME_SEC=0
    log_step "check all node's LFBs are in sync"
    while [ "$WAIT_TIME_SEC" != "$SYNC_TIMEOUT_SEC" ]; do
        if [ "$(do_read_lfb_hash '5')" = "$(do_read_lfb_hash '1')" ] && \
		[ "$(do_read_lfb_hash '4')" = "$(do_read_lfb_hash '1')" ] && \
		[ "$(do_read_lfb_hash '3')" = "$(do_read_lfb_hash '1')" ] && \
		[ "$(do_read_lfb_hash '2')" = "$(do_read_lfb_hash '1')" ]; then
	    log "all nodes in sync, proceeding..."
	    break
        fi
        WAIT_TIME_SEC=$((WAIT_TIME_SEC + 1))
        if [ "$WAIT_TIME_SEC" = "$SYNC_TIMEOUT_SEC" ]; then
            log "ERROR: Failed to confirm network sync"
            exit 1
        fi
        sleep 1
    done
}

function do_await_era_change() {
    # allow chain height to grow
    local ERA_COUNT=${1:-"1"}
    log_step "awaiting $ERA_COUNT eras…"
    await_n_eras "$ERA_COUNT"
}

function check_current_era {
    local ERA="null"
    while true; do
        ERA=$(get_chain_era $(get_node_for_dispatch) | awk '{print $NF}')
        if [ "$ERA" = "null" ] || [ "$ERA" = "N/A" ]; then
            sleep 1
        else
            break
        fi
    done
    echo "$ERA"
}

function check_faulty() {
    local NODE_ID=${1}
    local NODE_PATH=$(get_path_to_node $NODE_ID)
    grep -q 'this validator is faulty' "$NODE_PATH/logs/stdout.log"
    return $?
}

# Captures the public key in hex of a node minus the '01' prefix.
# This is because the logs don't include the '01' prefix when
# searching in them for the public key in hex. See itst14 for
# use case.
function get_node_public_key_hex() {
    local NODE_ID=${1}
    local NODE_PATH=$(get_path_to_node $NODE_ID)
    local PUBLIC_KEY_HEX=$(cat "$NODE_PATH"/keys/public_key_hex)
    echo "${PUBLIC_KEY_HEX:2}"
}

# Same as get_node_public_key_hex but includes the '01' prefix.
# When parsing the state with jq, the '01' prefix is included.
# See itst13 for use case.
function get_node_public_key_hex_extended() {
    local NODE_ID=${1}
    local NODE_PATH=$(get_path_to_node $NODE_ID)
    local PUBLIC_KEY_HEX=$(cat "$NODE_PATH"/keys/public_key_hex)
    echo "$PUBLIC_KEY_HEX"
}

function do_await_n_blocks() {
    local BLOCK_COUNT=${1:-1}
    log_step "Waiting $BLOCK_COUNT blocks..."
    nctl-await-n-blocks offset="$BLOCK_COUNT"
}
