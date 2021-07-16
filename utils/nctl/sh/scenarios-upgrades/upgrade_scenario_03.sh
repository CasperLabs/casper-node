#!/usr/bin/env bash
# ----------------------------------------------------------------
# Synopsis.
# ----------------------------------------------------------------

# 1. Start v1 running at current mainnet commit.
# 2. Waits for genesis era to complete.
# 3. Bonds in a non-genesis validator.
# 4. Delegates from an unused account.
# 5. Waits for the auction delay to take effect.
# 6. Asserts non-genesis validator is in auction info.
# 7. Asserts delegation is in auction info.
# 8. Stages the network for upgrade.
# 9. Assert v2 nodes run & the chain advances (new blocks are generated).
# 10. Waits 1 era.
# 11. Unbonds previously bonded non-genesis validator.
# 12. Undelegates from previously used account
# 13. Waits for the auction delay to take effect.
# 14. Asserts non-genesis validator is NO LONGER an active validator.
# 15. Asserts delegatee is NO LONGER in auction info.
# 16. Successful test cleanup.

# ----------------------------------------------------------------
# Imports.
# ----------------------------------------------------------------

source "$NCTL/sh/utils/main.sh"
source "$NCTL/sh/node/svc_$NCTL_DAEMON_TYPE".sh

# ----------------------------------------------------------------
# MAIN
# ----------------------------------------------------------------

# Main entry point.
function _main()
{
    local STAGE_ID=${1}

    if [ ! -d "$(get_path_to_stage "$STAGE_ID")" ]; then
        log "ERROR :: stage $STAGE_ID has not been built - cannot run scenario"
        exit 1
    fi

    _step_01 "$STAGE_ID"
    _step_02

    # Set initial protocol version for use later.
    INITIAL_PROTOCOL_VERSION=$(get_node_protocol_version 1)
    _step_03
    _step_04
    _step_05
    _step_06
    _step_07
    _step_08 "$STAGE_ID"
    _step_09 "$INITIAL_PROTOCOL_VERSION"
    _step_10
    _step_11
    _step_12
    _step_13
    _step_14
    _step_15
    _step_16
}

# Step 01: Start network from pre-built stage.
function _step_01()
{
    local STAGE_ID=${1}

    log_step_upgrades 1 "starting network from stage ($STAGE_ID)"

    source "$NCTL/sh/assets/setup_from_stage.sh" \
            stage="$STAGE_ID" \
            accounts_path="$NCTL/sh/scenarios/accounts_toml/upgrade_scenario_3.accounts.toml" \
            chainspec_path="$NCTL/sh/scenarios/chainspecs/upgrade_scenario_3.chainspec.toml.in"
    source "$NCTL/sh/node/start.sh" node=all
}

# Step 02: Await era-id >= 1.
function _step_02()
{
    log_step_upgrades 2 "awaiting genesis era completion"

    sleep 60.0
    await_until_era_n 1
}

# Step 03: Add bid non-genesis node and start
function _step_03()
{
    local NODE_ID=${1:-'6'}
    local TRUSTED_HASH

    TRUSTED_HASH="$(get_chain_latest_block_hash)"

    log_step_upgrades 3 "Submitting auction bid for node-$NODE_ID"
    if [ "$(get_node_is_up "$NODE_ID")" == false ]; then
        source "$NCTL/sh/contracts-auction/do_bid.sh" \
                node="$NODE_ID" \
                amount="$(get_node_staking_weight "$NODE_ID")" \
                rate="0" \
                quiet="FALSE"

        log "... Starting node-$NODE_ID with trusted_hash=$TRUSTED_HASH"
        do_node_start "$NODE_ID" "$TRUSTED_HASH"
    fi
}

# Step 04: Delegate from a nodes account
function _step_04()
{
    local NODE_ID=${1:-'5'}
    local ACCOUNT_ID=${2:-'7'}
    local AMOUNT=${3:-'1000'}

    log_step_upgrades 4 "Delegating $AMOUNT from account-$ACCOUNT_ID to validator-$NODE_ID"

    source "$NCTL/sh/contracts-auction/do_delegate.sh" \
            amount="$AMOUNT" \
            delegator="$ACCOUNT_ID" \
            validator="$NODE_ID"
}

# Step 05: Await 3 eras
function _step_05()
{
    log_step_upgrades 5 "Awaiting Auction_Delay = 3"
    await_n_eras '3' 'true' '5.0'
}

# Step 06: Assert NODE_ID is a validator
function _step_06()
{
    local NODE_ID=${1:-'6'}
    local NODE_PATH
    local HEX
    local AUCTION_INFO_FOR_HEX

    NODE_PATH=$(get_path_to_node "$NODE_ID")
    HEX=$(cat "$NODE_PATH"/keys/public_key_hex)
    AUCTION_INFO_FOR_HEX=$(nctl-view-chain-auction-info | jq --arg node_hex "$HEX" '.auction_state.bids[]| select(.public_key == $node_hex)')

    log_step_upgrades 6 "Asserting node-$NODE_ID is a validator"

    if [ ! -z "$AUCTION_INFO_FOR_HEX" ]; then
        log "... node-$NODE_ID found in auction info!"
        log "... public_key_hex: $HEX"
        echo "$AUCTION_INFO_FOR_HEX"
    else
        log "ERROR: Could not find $HEX in auction info!"
        exit 1
    fi
}

# Step 07: Assert USER_ID is a delegatee
function _step_07()
{
    local USER_ID=${1:-'7'}
    local USER_PATH
    local HEX
    local AUCTION_INFO_FOR_HEX

    USER_PATH=$(get_path_to_user "$USER_ID")
    HEX=$(cat "$USER_PATH"/public_key_hex)
    AUCTION_INFO_FOR_HEX=$(nctl-view-chain-auction-info | jq --arg node_hex "$HEX" '.auction_state.bids[]| select(.bid.delegators[].public_key == $node_hex)')

    log_step_upgrades 7 "Asserting user-$USER_ID is a delegatee"

    if [ ! -z "$AUCTION_INFO_FOR_HEX" ]; then
        log "... user-$USER_ID found in auction info delegators!"
        log "... public_key_hex: $HEX"
        echo "$AUCTION_INFO_FOR_HEX"
    else
        log "ERROR: Could not find $HEX in auction info delegators!"
        exit 1
    fi
}

# Step 08: Upgrade network from stage.
function _step_08()
{
    local STAGE_ID=${1}

    log_step_upgrades 8 "upgrading network from stage ($STAGE_ID)"

    log "... setting upgrade assets"
    source "$NCTL/sh/assets/upgrade_from_stage.sh" stage="$STAGE_ID" verbose=false

    log "... awaiting 2 eras + 1 block"
    await_n_eras '2' 'true' '5.0'
    await_n_blocks 1
}

# Step 09: Assert chain is progressing at all nodes.
function _step_09()
{
    local N1_PROTOCOL_VERSION_INITIAL=${1}
    local HEIGHT_1
    local HEIGHT_2
    local NODE_ID
    local TIMEOUT_SEC
    local UP_COUNT

    log_step_upgrades 9 "asserting node upgrades"

    TIMEOUT_SEC='0'

    while [ "$TIMEOUT_SEC" -le "60" ]; do
        UP_COUNT="$(get_count_of_up_nodes)"
        # Assert no nodes have stopped.
        if [ "$UP_COUNT" != "6" ]; then
            TIMEOUT_SEC=$((TIMEOUT_SEC + 1))
            log "TIMEOUT_SEC: $TIMEOUT_SEC"
            log "... $UP_COUNT != 6"
            sleep 1
        else
            log "... All nodes are up!"
            break
        fi

        if [ "$TIMEOUT_SEC" = '60' ]; then
            log "ERROR :: protocol upgrade failure - >= 1 nodes have stopped"
            log "... $UP_COUNT != 6"
            nctl-status
            exit 1
        fi
    done

    # Assert no nodes have stalled.
    HEIGHT_1=$(get_chain_height)
    await_n_blocks 2
    for NODE_ID in $(seq 1 "$(get_count_of_nodes)")
    do
        HEIGHT_2=$(get_chain_height "$NODE_ID")
        if [ "$HEIGHT_2" != "N/A" ] && [ "$HEIGHT_2" -le "$HEIGHT_1" ]; then
            log "ERROR :: protocol upgrade failure - >= 1 nodes have stalled"
            exit 1
        fi
    done

    # Assert node-1 protocol version incremented.
    N1_PROTOCOL_VERSION=$(get_node_protocol_version 1)
    if [ "$N1_PROTOCOL_VERSION" == "$N1_PROTOCOL_VERSION_INITIAL" ]; then
        log "ERROR :: protocol upgrade failure - >= protocol version did not increment"
        exit 1
    else
        log "Node 1 upgraded successfully: $N1_PROTOCOL_VERSION_INITIAL -> $N1_PROTOCOL_VERSION"
    fi

    # Assert nodes are running same protocol version.
    for NODE_ID in $(seq 2 "$(get_count_of_up_nodes)")
    do
        NX_PROTOCOL_VERSION=$(get_node_protocol_version "$NODE_ID")
        if [ "$NX_PROTOCOL_VERSION" != "$N1_PROTOCOL_VERSION" ]; then
            log "ERROR :: protocol upgrade failure - >= nodes are not all running same protocol version"
            exit 1
        else
            log "Node $NODE_ID upgraded successfully: $N1_PROTOCOL_VERSION_INITIAL -> $NX_PROTOCOL_VERSION"
        fi
    done
}

# Step 10: Await 1 era.
function _step_10()
{
    log_step_upgrades 10 "awaiting next era"
    await_n_eras '1' 'true' '5.0'
}

# Step 11: Unbond previously bonded validator
function _step_11()
{
    local NODE_ID=${1:-'6'}

    log_step_upgrades 11 "Submitting withdraw bid for node-$NODE_ID"
    if [ "$(get_node_is_up "$NODE_ID")" == true ]; then
        source "$NCTL/sh/contracts-auction/do_bid_withdraw.sh" \
                node="$NODE_ID" \
                amount="$(get_node_staking_weight "$NODE_ID")" \
                quiet="FALSE"
    fi
}

# Step 12: Undelegate previous user
function _step_12()
{
    local NODE_ID=${1:-'5'}
    local ACCOUNT_ID=${2:-'7'}
    local AMOUNT=${3:-'1000'}

    log_step_upgrades 12 "Undelegating $AMOUNT to account-$ACCOUNT_ID from validator-$NODE_ID"

    source "$NCTL/sh/contracts-auction/do_delegate_withdraw.sh" \
            amount="$AMOUNT" \
            delegator="$ACCOUNT_ID" \
            validator="$NODE_ID"
}

# Step 13: Await 3 eras
function _step_13()
{
    log_step_upgrades 13 "Awaiting Auction_Delay = 3"
    await_n_eras '3' 'true' '5.0'
}

# Step 14: Assert NODE_ID is NOT a validator
function _step_14()
{
    local NODE_ID=${1:-'6'}
    local NODE_PATH
    local HEX
    local AUCTION_INFO_FOR_HEX
    local INACTIVE_STATUS
    local STAKED_AMOUNT

    NODE_PATH=$(get_path_to_node "$NODE_ID")
    HEX=$(cat "$NODE_PATH"/keys/public_key_hex)
    AUCTION_INFO_FOR_HEX=$(nctl-view-chain-auction-info | jq --arg node_hex "$HEX" '.auction_state.bids[]| select(.public_key == $node_hex)')

    INACTIVE_STATUS=$(echo "$AUCTION_INFO_FOR_HEX" | grep 'inactive' | grep 'true' | sed 's/^ *//g')
    STAKED_AMOUNT=$(echo "$AUCTION_INFO_FOR_HEX" | grep 'staked_amount' | awk '{print $2}' | tr -d '[:punct:]')

    log_step_upgrades 14 "Asserting node-$NODE_ID is NOT a validator"

    if [ ! -z "$INACTIVE_STATUS" ]; then
        log "... node-$NODE_ID found as inactive! [expected]"
        log "... public_key_hex: $HEX"
        log "... $INACTIVE_STATUS"
    else
        log "ERROR: node-$NODE_ID found as active!"
        echo "$AUCTION_INFO_FOR_HEX"
        exit 1
    fi

    if [ "$STAKED_AMOUNT" = "0" ]; then
        log "... staked_amount: $STAKED_AMOUNT"
    else
        log "ERROR: node-$NODE_ID still has $STAKED_AMOUNT staked!"
        echo "$AUCTION_INFO_FOR_HEX"
        exit 1
    fi

    echo "$AUCTION_INFO_FOR_HEX"
}

# Step 07: Assert USER_ID is NOT a delegatee
function _step_15()
{
    local USER_ID=${1:-'7'}
    local USER_PATH
    local HEX
    local AUCTION_INFO_FOR_HEX

    USER_PATH=$(get_path_to_user "$USER_ID")
    HEX=$(cat "$USER_PATH"/public_key_hex)
    AUCTION_INFO_FOR_HEX=$(nctl-view-chain-auction-info | jq --arg node_hex "$HEX" '.auction_state.bids[]| select(.bid.delegators[].public_key == $node_hex)')

    log_step_upgrades 15 "Asserting user-$USER_ID is NOT a delegatee"

    if [ ! -z "$AUCTION_INFO_FOR_HEX" ]; then
        log "ERROR: user-$USER_ID found in auction info delegators!"
        log "... public_key_hex: $HEX"
        echo "$AUCTION_INFO_FOR_HEX"
        exit 1
    else
        log "... Could not find $HEX in auction info delegators! [expected]"
    fi
}

# Step 16: Terminate.
function _step_16()
{
    log_step_upgrades 16 "test successful - tidying up"

    source "$NCTL/sh/assets/teardown.sh"

    log_break
}

# ----------------------------------------------------------------
# ENTRY POINT
# ----------------------------------------------------------------

unset _STAGE_ID
unset INITIAL_PROTOCOL_VERSION

for ARGUMENT in "$@"
do
    KEY=$(echo "$ARGUMENT" | cut -f1 -d=)
    VALUE=$(echo "$ARGUMENT" | cut -f2 -d=)
    case "$KEY" in
        stage) _STAGE_ID=${VALUE} ;;
        *)
    esac
done

_main "${_STAGE_ID:-1}"
