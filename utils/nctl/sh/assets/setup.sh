#!/usr/bin/env bash
#
# Sets assets required to run an N node network.
# Arguments:
#   Network ordinal identifier.
#   Count of nodes to setup.
#   Count of nodes that will be bootstraps.
#   Count of users to setup.
#   Delay in seconds to apply to genesis timestamp.

#######################################
# Imports
#######################################

source "$NCTL"/sh/utils/main.sh
source "$NCTL"/sh/assets/setup_node.sh

#######################################
# Sets assets pertaining to network binaries.
# Globals:
#   NCTL_CASPER_HOME - path to node software github repo.
#   NCTL_CONTRACTS_SYSTEM - array of system contracts.
#   NCTL_CONTRACTS_CLIENT - array of client side contracts.
#######################################
function _set_bin()
{
    local PATH_TO_BIN

    # Set directory.
    PATH_TO_BIN=$(get_path_to_net)/bin
    mkdir "$PATH_TO_BIN"

    # Set executables.
    cp "$NCTL_CASPER_HOME"/target/release/casper-client "$PATH_TO_BIN"
    cp "$NCTL_CASPER_HOME"/target/release/casper-node "$PATH_TO_BIN"

    # Set system contracts.
	for CONTRACT in "${NCTL_CONTRACTS_SYSTEM[@]}"
	do
        cp "$NCTL_CASPER_HOME"/target/wasm32-unknown-unknown/release/"$CONTRACT" "$PATH_TO_BIN"
	done

    # Set client contracts.
	for CONTRACT in "${NCTL_CONTRACTS_CLIENT[@]}"
	do
        cp "$NCTL_CASPER_HOME"/target/wasm32-unknown-unknown/release/"$CONTRACT" "$PATH_TO_BIN"
	done
}

#######################################
# Sets assets pertaining to network chainspec.
# Arguments:
#   Delay in seconds to apply to genesis timestamp.
#######################################
function _set_chainspec()
{
    local GENESIS_DELAY=${1}
    local PATH_TO_NET
    local PATH_TO_CHAINSPEC
    local GENESIS_NAME
    local GENESIS_TIMESTAMP
    
    # Set directory.
    PATH_TO_NET=$(get_path_to_net)
    mkdir "$PATH_TO_NET"/chainspec

    # Set file.
    PATH_TO_CHAINSPEC=$PATH_TO_NET/chainspec/chainspec.toml
    cp "$NCTL_CASPER_HOME"/resources/local/chainspec.toml.in "$PATH_TO_CHAINSPEC"

    # Set config setting: genesis.name.
    GENESIS_NAME=$(get_chain_name)
    sed -i "s/casper-example/$GENESIS_NAME/g" "$PATH_TO_CHAINSPEC" > /dev/null 2>&1

    # Set config setting: genesis.timestamp.
    GENESIS_TIMESTAMP=$(get_genesis_timestamp "$GENESIS_DELAY")
    sed -i "s/^\([[:alnum:]_]*timestamp\) = .*/\1 = \"${GENESIS_TIMESTAMP}\"/" "$PATH_TO_CHAINSPEC" > /dev/null 2>&1

    # Override config settings as all paths need to point relative to nctl's assets dir:
    #    genesis.accounts_path
    sed -i "s?\${BASEDIR}/target/wasm32-unknown-unknown/release/?../bin/?g" "$PATH_TO_CHAINSPEC" > /dev/null 2>&1
    sed -i "s?\${BASEDIR}/resources/local/?./?g" "$PATH_TO_CHAINSPEC" > /dev/null 2>&1

    # Set accounts.csv.
    touch "$PATH_TO_NET"/chainspec/accounts.csv
}

#######################################
# Sets entry in chainspec's accounts.csv.
# Arguments:
#   Path to file containing an ed25519 public key in hex format.
#   Initial account balance (in motes).
#   Staking weight - validator's only (optional).
#######################################
function _set_chainspec_account()
{
    local PATH_TO_ACCOUNT_KEY=${1}
    local INITIAL_BALANCE=${2}
    local INITIAL_WEIGHT=${3:-0}
    local ACCOUNT_KEY
    local PATH_TO_NET
    
    PATH_TO_NET=$(get_path_to_net)

    ACCOUNT_KEY=$(cat "$PATH_TO_ACCOUNT_KEY")
	cat >> "$PATH_TO_NET"/chainspec/accounts.csv <<- EOM
	${ACCOUNT_KEY},$INITIAL_BALANCE,$INITIAL_WEIGHT
	EOM
}

#######################################
# Sets assets pertaining to network daemon.
# Globals:
#   NCTL - path to nctl home directory.
#   NCTL_DAEMON_TYPE - type of daemon service manager.
#######################################
function _set_daemon()
{
    local PATH_TO_NET

    # Set directory.
    PATH_TO_NET=$(get_path_to_net)
    mkdir "$PATH_TO_NET"/daemon
    mkdir "$PATH_TO_NET"/daemon/config
    mkdir "$PATH_TO_NET"/daemon/logs
    mkdir "$PATH_TO_NET"/daemon/socket

    # Set daemon specific artefacts.
    if [ "$NCTL_DAEMON_TYPE" = "supervisord" ]; then
        source "$NCTL"/sh/assets/setup_supervisord.sh
    fi
}

#######################################
# Sets assets pertaining to network faucet account.
# Arguments:
#   Path to network directory.
#######################################
function _set_faucet()
{
    local PATH_TO_NET

    # Set directory.
    PATH_TO_NET=$(get_path_to_net)
    mkdir "$PATH_TO_NET"/faucet

    # Set keys.
    "$PATH_TO_NET"/bin/casper-client keygen -f "$PATH_TO_NET"/faucet > /dev/null 2>&1

    # Set chainspec account.
    _set_chainspec_account \
        "$PATH_TO_NET"/faucet/public_key_hex \
        "$NCTL_INITIAL_BALANCE_FAUCET"
}

#######################################
# Sets assets pertaining to all nodes within network.
# Arguments:
#   Count of genesis nodes to setup.
#######################################
function _set_nodes()
{
    local COUNT_GENESIS_NODES=${1}
    local PATH_TO_NET

    # Set directory.
    PATH_TO_NET=$(get_path_to_net)
    mkdir "$PATH_TO_NET"/nodes

    # We setup assets for twice the number of genesis nodes so that we can subsequently rotate them.
    for NODE_ID in $(seq 1 $((COUNT_GENESIS_NODES * 2)))
    do
        setup_node "$NODE_ID" "$COUNT_GENESIS_NODES"
    done
}

#######################################
# Sets assets pertaining to all users within network.
# Arguments:
#   Count of users to setup.
#######################################
function _set_users()
{
    local COUNT_USERS=${1}
    local PATH_TO_NET

    # Set directory.
    PATH_TO_NET=$(get_path_to_net)
    mkdir "$PATH_TO_NET"/users

    for USER_ID in $(seq 1 "$COUNT_USERS")
    do
        "$PATH_TO_NET"/bin/casper-client keygen -f "$PATH_TO_NET"/users/user-"$USER_ID" > /dev/null 2>&1
        _set_chainspec_account \
            "$PATH_TO_NET"/users/user-"$USER_ID"/public_key_hex \
            "$NCTL_INITIAL_BALANCE_VALIDATOR"
    done
}

#######################################
# Main
# Globals:
#   NET_ID - ordinal identifier of network being setup.
# Arguments:
#   Count of nodes to setup.
#   Count of users to setup.
#   Delay in seconds to apply to genesis timestamp.
#######################################
function _main()
{
    local COUNT_NODES=${1}
    local COUNT_USERS=${2}
    local GENESIS_DELAY=${3}
    local PATH_TO_NET

    # Tear down previous.
    PATH_TO_NET=$(get_path_to_net)
    if [ -d "$PATH_TO_NET" ]; then
        source "$NCTL"/sh/assets/teardown.sh net="$NET_ID"
    fi
    mkdir -p "$PATH_TO_NET"

    log "net-$NET_ID: asset setup begins ... please wait"

    # Set artefacts.    
    log "... setting binaries"    
    _set_bin

    log "... setting chainspec"
    _set_chainspec "$GENESIS_DELAY"

    log "... setting faucet"
    _set_faucet 

    log "... setting nodes"
    _set_nodes "$COUNT_NODES"

    log "... setting users"
    _set_users "$COUNT_USERS"

    log "... setting daemon"
    _set_daemon

    log "net-$NET_ID: asset setup complete"
}

#######################################
# Destructure input args.
#######################################

unset GENESIS_DELAY_SECONDS
unset NET_ID
unset NODE_COUNT
unset USER_COUNT

for ARGUMENT in "$@"
do
    KEY=$(echo "$ARGUMENT" | cut -f1 -d=)
    VALUE=$(echo "$ARGUMENT" | cut -f2 -d=)
    case "$KEY" in
        delay) GENESIS_DELAY_SECONDS=${VALUE} ;;
        net) NET_ID=${VALUE} ;;
        nodes) NODE_COUNT=${VALUE} ;;
        users) USER_COUNT=${VALUE} ;;
        *)
    esac
done


export NET_ID=${NET_ID:-1}
GENESIS_DELAY_SECONDS=${GENESIS_DELAY_SECONDS:-30}
NODE_COUNT=${NODE_COUNT:-5}
USER_COUNT=${USER_COUNT:-5}

if [ 3 -gt "$NODE_COUNT" ]; then
    log_error "Invalid input: |nodes| MUST BE >= 3"
else
    _main "$NODE_COUNT" "$USER_COUNT" "$GENESIS_DELAY_SECONDS"
fi
