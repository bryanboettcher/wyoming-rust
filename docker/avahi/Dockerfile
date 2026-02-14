FROM alpine:3.21
RUN apk add --no-cache avahi
COPY wyoming.service /etc/avahi/services/wyoming.service
COPY entrypoint.sh /entrypoint.sh
ENTRYPOINT ["/entrypoint.sh"]
