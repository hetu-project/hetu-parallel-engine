# hetu-hub-consensus

## Deployment Guide

### Building and Setting Up Nodes

1. First, build the node in the node directory:
   ```bash
   cd node
   cargo build --release
   ```

2. Generate keys for each node:
   ```bash
   ./node generate_keys --filename node0.json
   ```
   Repeat this step for each node you want to deploy.

3. Update the committee configuration:
   - Make a copy of `.committee.json.example`
   - Update it with the new node IDs and IP addresses for each node
   - Save it as `committee.json`

### Running the Nodes

#### Start Worker Node
```bash
./node -vv run --keys node.json --committee committee.json --store .db-worker --parameters parameters.json worker --id 0
```

#### Start Primary Node
```bash
./node -vv run --keys node.json --committee committee.json --store .db-primary-0 --parameters parameters.json primary
```

**Important**: 
- Each node (primary and worker) should be run on a different machine
- Each node will have its own unique `node.json` file generated locally
- All nodes share the same `committee.json` and `parameters.json` files
