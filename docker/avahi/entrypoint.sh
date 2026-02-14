#!/bin/sh
set -e

# Template the service file with environment variables
SATELLITE_NAME="${SATELLITE_NAME:-Wyoming Satellite}"
SATELLITE_PORT="${SATELLITE_PORT:-10700}"

sed -i "s|SATELLITE_NAME|${SATELLITE_NAME}|g" /etc/avahi/services/wyoming.service
sed -i "s|SATELLITE_PORT|${SATELLITE_PORT}|g" /etc/avahi/services/wyoming.service

exec avahi-daemon --no-drop-root
