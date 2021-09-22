FROM rust:1.55.0-slim-buster

RUN apt-get update && \
    apt-get install -y openssh-client && \
    rm -rf /var/cache/apt/archives /var/cache/apt/lists
