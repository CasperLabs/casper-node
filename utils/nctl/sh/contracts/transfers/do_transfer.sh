#!/usr/bin/env bash

#######################################
# Imports
#######################################

source $NCTL/sh/utils.sh

#######################################
# Dispatches native transfers to a test net.
# Arguments:
#   Network ordinal identifier.
#   Node ordinal identifier.
#   Transfer amount.
#   User ordinal identifier.
#   Count of transfers to be dispatched.
#   Transfer dispatch interval.
#   Gas price.
#   Gas payment.
#######################################
function do_transfer_native()
{
    local NET_ID=${1}
    local NODE_ID=${2}
    local AMOUNT=${3}
    local USER_ID=${4}
    local TRANSFERS=${5}
    local TRANSFER_INTERVAL=${6}
    local GAS=${7}
    local PAYMENT=${8}

    local CHAIN_NAME=$(get_chain_name $NET_ID)
    local CP1_SECRET_KEY=$(get_path_to_secret_key $NET_ID $NCTL_ACCOUNT_TYPE_FAUCET)
    local CP1_PUBLIC_KEY=$(get_account_key $NET_ID $NCTL_ACCOUNT_TYPE_FAUCET)
    local CP2_PUBLIC_KEY=$(get_account_key $NET_ID $NCTL_ACCOUNT_TYPE_USER $USER_ID)
    local PATH_TO_CLIENT=$(get_path_to_client $NET_ID)

    log "dispatching $TRANSFERS native transfers"
    log "... network=$NET_ID"
    log "... node=$NODE_ID"
    log "... transfer amount=$AMOUNT"
    log "... transfer interval=$TRANSFER_INTERVAL (s)"
    log "... counter-party 1 public key=$CP1_PUBLIC_KEY"
    log "... counter-party 2 public key=$CP2_PUBLIC_KEY"
    log "... dispatched deploys:"

    function _dispatch_deploy {
        echo $(
            $PATH_TO_CLIENT transfer \
                --chain-name $CHAIN_NAME \
                --gas-price $GAS \
                --node-address $NODE_ADDRESS \
                --payment-amount $PAYMENT \
                --secret-key $CP1_SECRET_KEY \
                --ttl "1day" \
                --amount $AMOUNT \
                --target-account $CP2_PUBLIC_KEY \
                | jq '.result.deploy_hash' \
                | sed -e 's/^"//' -e 's/"$//'
            )
    }

    # Round robin dispatch.
    if [ $NODE_ID = "all" ]; then
        local IDX
        local NODE_ADDRESS
        local TRANSFERRED=0
        while [ $TRANSFERRED -lt $TRANSFERS ];
        do
            for NODE_ID in $(seq 1 $(get_count_of_genesis_nodes $NET_ID))
            do
                NODE_ADDRESS=$(get_node_address_rpc $NET_ID $NODE_ID)
                DEPLOY_HASH=$(_dispatch_deploy)
                TRANSFERRED=$((TRANSFERRED + 1))
                log "... ... #$TRANSFERRED :: $DEPLOY_HASH"
                if [[ $TRANSFERRED -eq $TRANSFERS ]]; then
                    break
                fi
                sleep $TRANSFER_INTERVAL
            done
        done

    # Specific node dispatch.
    else
        local NODE_ADDRESS=$(get_node_address_rpc $NET_ID $NODE_ID)
        for TRANSFER_ID in $(seq 1 $TRANSFERS)
        do
            DEPLOY_HASH=$(_dispatch_deploy)
            log "... ... #$TRANSFER_ID :: $DEPLOY_HASH"
            sleep $TRANSFER_INTERVAL
        done
    fi

    log "dispatched $TRANSFERS native transfers"
}

#######################################
# Destructure input args.
#######################################

unset AMOUNT
unset GAS
unset TRANSFER_INTERVAL
unset NET_ID
unset NODE_ID
unset PAYMENT
unset TRANSFERS
unset USER_ID

for ARGUMENT in "$@"
do
    KEY=$(echo $ARGUMENT | cut -f1 -d=)
    VALUE=$(echo $ARGUMENT | cut -f2 -d=)
    case "$KEY" in
        amount) AMOUNT=${VALUE} ;;
        gas) GAS=${VALUE} ;;
        interval) TRANSFER_INTERVAL=${VALUE} ;;
        net) NET_ID=${VALUE} ;;
        node) NODE_ID=${VALUE} ;;
        payment) PAYMENT=${VALUE} ;;
        transfers) TRANSFERS=${VALUE} ;;
        user) USER_ID=${VALUE} ;;
        *)
    esac
done

do_transfer_native \
    ${NET_ID:-1} \
    ${NODE_ID:-1} \
    ${AMOUNT:-$NCTL_DEFAULT_TRANSFER_AMOUNT} \
    ${USER_ID:-1} \
    ${TRANSFERS:-100} \
    ${TRANSFER_INTERVAL:-0.01} \
    ${GAS:-$NCTL_DEFAULT_GAS_PRICE} \
    ${PAYMENT:-$NCTL_DEFAULT_GAS_PAYMENT}
