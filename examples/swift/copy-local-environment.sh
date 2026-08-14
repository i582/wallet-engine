#!/bin/sh

set -eu

resource_path="${TARGET_BUILD_DIR}/${UNLOCALIZED_RESOURCES_FOLDER_PATH}/toncenter-api-key"

# A provider key embedded in an application bundle is not a production secret.
# Package the local key only for Debug builds of this example application.
if [ "${CONFIGURATION}" != "Debug" ]; then
    rm -f "${resource_path}"
    exit 0
fi

environment_path="${SRCROOT}/.env"
if [ ! -f "${environment_path}" ]; then
    rm -f "${resource_path}"
    echo "warning: ${environment_path} is absent; Toncenter requests will be unauthenticated"
    exit 0
fi

api_key="$(sed -n 's/^[[:space:]]*TONCENTER_API_KEY[[:space:]]*=[[:space:]]*//p' "${environment_path}" | tail -n 1 | tr -d '\r')"
if [ -z "${api_key}" ]; then
    # Keep existing local environments working while they migrate to the
    # network-neutral variable name.
    api_key="$(sed -n 's/^[[:space:]]*TONCENTER_TESTNET_API_KEY[[:space:]]*=[[:space:]]*//p' "${environment_path}" | tail -n 1 | tr -d '\r')"
fi
api_key="$(printf '%s' "${api_key}" | sed -e 's/^"//' -e 's/"$//' -e "s/^'//" -e "s/'$//")"

if [ -z "${api_key}" ]; then
    rm -f "${resource_path}"
    echo "warning: TONCENTER_API_KEY is empty; Toncenter requests will be unauthenticated"
    exit 0
fi

umask 077
mkdir -p "$(dirname "${resource_path}")"
printf '%s' "${api_key}" > "${resource_path}"
