# Hetu Parallel Engine

Hetu Parallel Engine is a high-performance blockchain consensus engine that combines the power of CometBFT with a customized DAG-based Narwhal consensus implementation.

## Architecture

The engine consists of two main components:

- **hetu-cometbft**: A customized version of CometBFT based on Cosmos-SDK
- **hetu-consensus**: A DAG-based consensus implementation derived from Narwhal

### System Design

```ascii:/README.md
+----------------+        +------------------+
|  hetu-cometbft |  <---> |  hetu-consensus  |
|   (CometBFT)   |        |    (Narwhal)     |
+----------------+        +------------------+

The two components communicate via socket connection, where hetu-consensus (Narwhal) handles the ordering consensus.
```
### Features
High-throughput consensus mechanism
DAG-based parallel transaction processing
Compatible with Cosmos-SDK ecosystem
Customized CometBFT implementation
Socket-based inter-component communication
Optimized for performance and scalability
## Components
### hetu-cometbft
A fork of CometBFT with customizations for:

Integration with Narwhal consensus
Enhanced performance optimizations
Additional functionality for the Hetu ecosystem
### hetu-consensus
A customized implementation of the Narwhal consensus protocol featuring:

DAG-based transaction ordering
High-throughput processing
Optimized network communication
## Getting Started

### Prerequisites
- Go 1.19 or later
- Rust 1.70 or later

## Contributing
We welcome contributions! Please read our contributing guidelines before submitting pull requests.

### License
This project is licensed under [LICENSE NAME] - see the LICENSE file for details.

### Acknowledgments
CometBFT Team
Cosmos-SDK Team
Narwhal Team at Meta (formerly Facebook)
