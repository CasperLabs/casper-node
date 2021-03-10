#!/usr/bin/env bash
set -e

# Meant to run only in CI
if [ -z "${DRONE}" ]; then
  echo "Must be run on Drone!"
  exit 1
fi

DRONE_ROOT_DIR="/drone/src"
SCENARIOS_DIR="$DRONE_ROOT_DIR/utils/nctl/sh/scenarios"
SCENARIOS_CHAINSPEC_DIR="$SCENARIOS_DIR/chainspecs"
LAUNCHER_DIR="/drone"

# NCTL requires casper-node-launcher
pushd $LAUNCHER_DIR
git clone https://github.com/CasperLabs/casper-node-launcher.git

# Activate Environment
pushd $DRONE_ROOT_DIR
source $(pwd)/utils/nctl/activate
# Build, Setup, and Start NCTL
nctl-compile

function start_run_teardown() {
    local RUN_CMD=$1
    local RUN_CHAINSPEC=$2
    echo "Setting up network: $RUN_CMD $RUN_CHAINSPEC"
    if [ -z "$RUN_CHAINSPEC" ]; then
        nctl-assets-setup
    else
        nctl-assets-setup chainspec_path="$SCENARIOS_CHAINSPEC_DIR/$RUN_CHAINSPEC"
    fi
    sleep 1
    nctl-start
    echo "Sleeping 90 to allow network startup"
    sleep 90
    pushd $SCENARIOS_DIR
    # Don't qoute the cmd
    echo "Starting scenario: $RUN_CMD $RUN_CHAINSPEC"
    source $RUN_CMD
    popd
    nctl-assets-teardown
    sleep 1
}

start_run_teardown "sync_test.sh node=6 timeout=500"
start_run_teardown "sync_upgrade_test.sh node=6 timeout=500"
start_run_teardown "itst01.sh"
start_run_teardown "itst02.sh"
start_run_teardown "itst11.sh"
start_run_teardown "itst13.sh" "itst13.chainspec.toml.in"

# Clean up cloned repo
popd
echo "Removing $LAUNCHER_DIR/casper-node-launcher"
rm -rf "$LAUNCHER_DIR/casper-node-launcher"
