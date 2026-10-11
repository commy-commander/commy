---
title: P2P architecture
description: How iroh connects peers directly, and what relays actually do.
order: 4
---

Commy is built on [iroh](https://iroh.computer), a Rust library for direct peer to peer connections.

## Identities are keypairs

Every device running Commy holds an Ed25519 keypair. The public key is the iroh node id: a stable, verifiable address for that device. When you link a device to an account, the app proves it holds the private key by signing a challenge.

## Direct QUIC connections

Peers talk over QUIC: encrypted, multiplexed and fast. Voice packets, chat messages and file chunks travel straight from one member's machine to another's.

## Hole punching

Most home networks sit behind NAT. iroh coordinates through a relay so both peers learn each other's public address, then opens a direct path. In most cases this succeeds and traffic flows peer to peer.

## Relays are a fallback

Relay servers help with discovery and carry traffic only when a direct path cannot be made, for example on strict corporate networks. Relayed traffic stays end to end encrypted, so the relay only sees ciphertext.

## Servers live on members' machines

A Commy server is replicated state held by its members. Channels, history, emojis and soundboards sync between members' devices when they are online, and files shared in a server reach only that server's members. There is no central host to pay for or shut down.

## What an account server does

The only hosted pieces are the website, optional accounts that link your devices, and invite links. None of them see your messages, calls or files. See [Privacy](/docs/privacy) for the full list.
