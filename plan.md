Project Brief: Kaspa Network Analysis Sensor
1. Objective

The goal of this project is to develop a lightweight, distributed network sensor to map the topology of the Kaspa peer-to-peer (P2P) network. The primary purpose is to empirically identify and classify all connecting nodes as either Public (Listening) or Private (Non-Listening). This will allow us to gather high-fidelity data on the true size and composition of the network.

The final product will be a scalable application, deployable as a fleet of sensors across various global Autonomous Systems (ASNs), with all collected data aggregated into a central database for analysis.

2. Core Concept: The "Protocol Skeleton" Node

Instead of running a full kaspad node, which is resource-intensive, we will build a stripped-down application that acts as a "protocol skeleton." This application will not participate in the network's consensus, sync the DAG, or relay transactions. Its sole responsibilities are to listen for connections, perform the initial P2P handshake, log the interaction, and classify the peer.

By stripping out all non-essential functionalities, the resulting sensor will be extremely lightweight, allowing for cost-effective deployment at a massive scale. The engineer should start with the rusty-kaspa repository, leveraging its existing P2P libraries and protocol definitions to handle the network communication layer.

3. Functional Requirements

The project will be developed in three main phases.

Phase 1: Passive Listener & Data Collection
The initial version of the sensor will act as a passive data collector.

Network Listener:

The application must open a TCP socket and listen for inbound connections on the standard Kaspa P2P port (16111).   

P2P Handshake Protocol:

The sensor must correctly implement the initial Kaspa P2P handshake to appear as a valid peer. Kaspa's P2P layer is built on gRPC and Protocol Buffers (protobufs).   

It must successfully receive and parse the connecting peer's version message.

It must respond with its own valid version and verack messages to complete the handshake.   

The connection can be terminated after the handshake and logging are complete.

Data Logging & Transmission:

For every successful handshake, the sensor must log the following information:

Timestamp: High-precision UTC timestamp of the connection.

Peer IP Address & Port: The source address of the connecting peer.

Sensor ID: A configurable identifier for the sensor instance (e.g., aws-us-east-1, ASN-12345).

Peer Metadata: Key information extracted from the peer's version message, including:

protocolVersion

userAgent string

This logged data must be transmitted securely to a centralized data store (e.g., Google Firebase/Firestore).

Phase 2: Active Probe & Peer Classification
The second phase builds upon the first by adding an active analysis component.

Active Probe Mechanism:

Immediately following a successful inbound handshake from a peer at a given IP:Port, the sensor must trigger an "active probe."

This probe consists of initiating a new, independent outbound TCP connection back to the connecting peer's IP:Port.

The probe should have a short, aggressive timeout (e.g., < 1 second) to determine reachability quickly.

Peer Classification Logic:

The outcome of the active probe will determine the peer's classification:

Probe Success: If the TCP connection is successfully established, the peer is classified as Public.

Probe Failure: If the connection times out or is refused, the peer is classified as Private.

Enhanced Logging:

The log entry for each connection must be augmented to include this classification status (Public or Private).

Phase 3: Network Presence & Gossip Analysis
Before large-scale deployment, it is critical to investigate the sensor's visibility within the Kaspa P2P network. The objective of this phase is to determine if a "protocol skeleton" node will be added to the peer lists of other nodes and subsequently shared across the network through the standard peer gossip mechanism.

Research Question: Will a node that only completes the P2P handshake (and does not relay blocks, transactions, or respond to data requests) be considered an "active" peer by other nodes and have its address propagated via addr messages?

Investigation Protocol:

Deploy a single instance of the Phase 1 sensor node and allow it to run for a sustained period (e.g., 24-48 hours), accepting inbound connections and completing handshakes.

During this period, use a separate, standard Kaspa node (the "crawler") to actively query the network.

The crawler should connect to a diverse set of public peers and send getaddr messages to them.   

Analyze the addr messages received by the crawler. The primary goal is to determine if the IP address of the sensor node appears in the peer lists provided by other nodes in the network.

Success Criteria & Reporting:

The deliverable for this phase is a report answering the research question.

The report should quantify the findings (e.g., "The sensor's IP was observed in X% of addr responses from Y unique peers after Z hours").

This analysis will inform our deployment strategy. If sensors are gossiped, their reach will grow organically. If not, we will need to rely on other discovery mechanisms to ensure the sensor fleet receives a representative sample of inbound connections.

4. Architectural & Deployment Guidelines

Codebase: Use the rusty-kaspa repository as the foundation. Extract and utilize the necessary modules for P2P communication, message serialization (protobufs), and network address management. All other components (consensus engine, DAG storage, RPC server, etc.) should be removed.

Lightweight Design: The final application should have minimal CPU, memory, and disk footprint. It is a network utility, not a blockchain node.

Containerization: The application should be packaged into a Docker container for easy, repeatable, and scalable deployment across different hosting environments.

Distributed Deployment: The architecture must support running hundreds of instances simultaneously. Each instance will be configured with its unique Sensor ID to allow for geographic and topological analysis of the aggregated data.

5. Project Scope & Exclusions

IN SCOPE:

A lightweight Rust application that performs the functions described in Phases 1, 2, and 3.

Configuration for deployment in a Docker container.

Integration with a specified centralized database for data transmission.

OUT OF SCOPE:

Implementation of any Kaspa consensus rules.

Storage or validation of the Kaspa DAG.

Development of the data analysis backend or any visualization dashboards.

Management of the cloud infrastructure for deployment.