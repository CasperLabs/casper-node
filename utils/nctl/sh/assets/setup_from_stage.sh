#!/usr/bin/env bash

#######################################
# Prepares assets for network start.
# Arguments:
#   Stage ordinal identifier.
#   Network ordinal identifier.
#   Network nodeset count.
#   Delay in seconds pripr to which genesis window will expire.
#   Path to custom accounts.toml.
#######################################

source "$NCTL/sh/utils/main.sh"
source "$NCTL/sh/assets/setup_shared.sh"

# ----------------------------------------------------------------
# MAIN
# ----------------------------------------------------------------

#######################################
# Prepares assets for network start.
# Arguments:
#   Stage ordinal identifier.
#   Network nodeset count at genesis.
#   Delay in seconds pripr to which genesis window will expire.
#   Path to custom accounts.toml.
#######################################
function _main()
{
    log "setup of assets from stage ${1}: STARTS"

    local STAGE_ID=${1}
    local COUNT_NODES_AT_GENESIS=${2}
    local COUNT_NODES=$((COUNT_NODES_AT_GENESIS * 2))
    local GENESIS_DELAY=${3}
    local PATH_TO_ACCOUNTS=${4}
    local PATH_TO_CHAINSPEC=${5}
    local COUNT_USERS="$COUNT_NODES"
    local PATH_TO_NET
    local PATH_TO_STAGE
    local PATH_TO_STAGED_ASSETS
    local PROTOCOL_VERSION
    local PROTOCOL_VERSION_FS

    PATH_TO_NET=$(get_path_to_net)
    PROTOCOL_VERSION=$(_get_initial_protocol_version "$STAGE_ID")
    PROTOCOL_VERSION_FS=$(get_protocol_version_for_fs "$PROTOCOL_VERSION")
    PATH_TO_STAGED_ASSETS="$(get_path_to_stage "$STAGE_ID")/$PROTOCOL_VERSION_FS"

    # Tear down previous.
    if [ -d "$PATH_TO_NET" ]; then
        source "$NCTL/sh/assets/teardown.sh" net="$NET_ID"
    fi
    mkdir -p "$PATH_TO_NET"

    # Setup new.
    setup_asset_directories "$COUNT_NODES" "$COUNT_USERS" "$PROTOCOL_VERSION_FS"
    setup_asset_binaries "$PROTOCOL_VERSION_FS" \
                         "$COUNT_NODES" \
                         "$PATH_TO_STAGED_ASSETS/casper-client" \
                         "$PATH_TO_STAGED_ASSETS/casper-node" \
                         "$PATH_TO_STAGED_ASSETS/casper-node-launcher" \
                         "$PATH_TO_STAGED_ASSETS"
    setup_asset_keys "$COUNT_NODES" "$COUNT_USERS"
    setup_asset_daemon

    if [ ! -z "$PATH_TO_CHAINSPEC" ]; then
        for subdir in $(find $(get_path_to_stage "$STAGE_ID")/* -type d); do
            cp "$PATH_TO_CHAINSPEC" "$subdir/chainspec.toml"
        done
    fi

    setup_asset_chainspec "$COUNT_NODES" \
                          "$PROTOCOL_VERSION" \
                          $(get_genesis_timestamp "$GENESIS_DELAY") \
                          "$PATH_TO_STAGED_ASSETS/chainspec.toml" \
                          true

    if [ "$PATH_TO_ACCOUNTS" = "" ]; then
        setup_asset_accounts "$COUNT_NODES" \
                             "$COUNT_NODES_AT_GENESIS" \
                             "$COUNT_USERS"
    else
        setup_asset_accounts_from_template "$COUNT_NODES" \
                                           "$COUNT_USERS" \
                                           "$PATH_TO_ACCOUNTS"
    fi
    setup_asset_node_configs "$COUNT_NODES" \
                             "$PROTOCOL_VERSION_FS" \
                             "$PATH_TO_STAGED_ASSETS/config.toml" \
                             true

    log "setup of assets from stage ${1}: COMPLETE"
}

#######################################
# Returns initial protocol version from which assets will be setup.
#######################################
function _get_initial_protocol_version()
{
    local STAGE_ID=${1}

    for FHANDLE in "$(get_path_to_stage "$STAGE_ID")/"*; do        
        if [ -d "$FHANDLE" ]; then
            echo "$(basename "$FHANDLE")" | tr "_" "."
            break
        fi
    done
}

# ----------------------------------------------------------------
# ENTRY POINT
# ----------------------------------------------------------------

unset PATH_TO_ACCOUNTS
unset GENESIS_DELAY_SECONDS
unset NET_ID
unset NODE_COUNT
unset STAGE_ID
unset PATH_TO_CHAINSPEC

for ARGUMENT in "$@"
do
    KEY=$(echo "$ARGUMENT" | cut -f1 -d=)
    VALUE=$(echo "$ARGUMENT" | cut -f2 -d=)
    case "$KEY" in
        accounts_path) PATH_TO_ACCOUNTS=${VALUE} ;;
        delay) GENESIS_DELAY_SECONDS=${VALUE} ;;
        net) NET_ID=${VALUE} ;;
        nodes) NODE_COUNT=${VALUE} ;;
        stage) STAGE_ID=${VALUE} ;;
        chainspec_path) PATH_TO_CHAINSPEC=${VALUE} ;;
        *)
    esac
done

export NET_ID=${NET_ID:-1}
GENESIS_DELAY_SECONDS=${GENESIS_DELAY_SECONDS:-30}
NODE_COUNT=${NODE_COUNT:-5}
PATH_TO_ACCOUNTS=${PATH_TO_ACCOUNTS:-""}
STAGE_ID="${STAGE_ID:-1}"
PATH_TO_CHAINSPEC=${PATH_TO_CHAINSPEC:-""}

_main "$STAGE_ID" \
      "$NODE_COUNT" \
      "$GENESIS_DELAY_SECONDS" \
      "$PATH_TO_ACCOUNTS" \
      "$PATH_TO_CHAINSPEC"
