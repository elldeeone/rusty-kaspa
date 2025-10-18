# Tor Integration Research Validation & Extension Prompt

## Mission

You are a research engineer tasked with validating and extending our Tor integration research for rusty-kaspa (a Rust blockchain node implementation). We've done initial research documented in `/docs/tor-integration-research.md` and need you to:

1. **Validate** our findings with current (Oct 2025) information
2. **Identify gaps** in our understanding
3. **Discover edge cases** and gotchas we may have missed
4. **Find production examples** of similar integrations
5. **Assess risks** and potential blockers

## Background Context

**Project**: Implementing Tor support for rusty-kaspa blockchain node (similar to Bitcoin Core's Tor integration)

**Goals**:
- Route outbound P2P connections through Tor SOCKS5 proxy
- Publish node as Tor hidden service (.onion address)
- Support .onion peer addresses in address book
- Enable privacy-focused users to run nodes without clearnet exposure

**Current Stack**:
- Rust (async: Tokio, gRPC: Tonic 0.12.3, HTTP: Hyper 1.x)
- P2P networking over gRPC bidirectional streaming
- RocksDB for peer address storage

## Research Areas

### 1. Rust Tor Ecosystem Validation (CRITICAL)

**Arti (Official Rust Tor Implementation)**

Research Questions:
- What is the **current production status** of Arti as of Oct 2025? (Latest version, MSRV, stability)
- What is the **actual state of onion service support** in Arti? Our research found conflicting info:
  - We found Arti 1.0.0 declared "production ready"
  - Arti 1.3.0 (Oct 2024) claimed "client feature parity"
  - Arti 1.4.x-1.6.x have onion service work
  - BUT Feb 2024 docs said "features missing for secure onion services"
  - **CRITICAL**: Can we use Arti for onion services TODAY (Oct 2025) or not?
- Is `tor-hsservice` crate production-ready for creating .onion services?
- What are known bugs, limitations, or security concerns with Arti onion services?
- Are there any production projects using Arti for onion services successfully?
- Memory/performance characteristics vs C Tor?
- What features are still missing vs C Tor?

**SOCKS5 Libraries**

We identified 4 candidates. Please validate:
- `tokio-socks`: Still most popular? Any known issues with Tonic/Hyper integration?
- `hyper-socks2`: Is it compatible with Hyper 1.x AND Tonic 0.12.3? Last update Mar 2024 - abandoned?
- `fast-socks5`: Any production usage examples? Performance benchmarks?
- `tor-socksproto`: Can this be used standalone or only within Arti ecosystem?

**Specific Research**:
- Search for: "rust tonic socks5 proxy connector" - find working examples
- Search for: "tokio-socks hyper 1.0 compatibility"
- Find: GitHub repos using tokio-socks with Tonic gRPC (production examples)
- Find: Any Tower middleware for SOCKS proxy support

**tor-interface Crate**

- Current version and maintenance status (we found v0.6.0, Oct 2025)
- Production usage examples - who uses it?
- Cross-platform reliability (Linux/macOS/Windows)
- Does it work with latest Tor daemon versions?
- Any known issues with control port authentication?
- Alternative libraries for Tor control protocol in Rust?

### 2. Bitcoin Core Tor Integration Deep Dive

We have high-level understanding but need details:

**Implementation Details**:
- Read Bitcoin Core source code (if available publicly) - how EXACTLY do they:
  - Create the SOCKS connector?
  - Handle stream isolation (what SOCKS username format)?
  - Manage Tor daemon lifecycle?
  - Store and serialize .onion addresses?
  - Handle network type discrimination (clearnet vs onion)?
  - Implement `--onlynet=onion` filtering?
  - Prevent IP leakage in Tor mode?

**Configuration & UX**:
- Full list of ALL Tor-related flags in Bitcoin Core (not just the main ones)
- Default values for each flag
- Interactions between flags (e.g., what happens if you set both `-proxy` and `-onion`?)
- Error messages users see when Tor is misconfigured
- How does Bitcoin Core handle Tor daemon not running?
- Bootstrap sequence timing - how long does it wait for Tor?

**Find**:
- Bitcoin Core Tor integration documentation (official docs, wiki pages, blog posts)
- User guides for running Bitcoin Core over Tor
- Common issues/pitfalls users encounter
- Best practices for production deployments

### 3. Other Blockchain Nodes with Tor Support

Research other cryptocurrency/blockchain nodes with Tor integration:

**Find Projects**:
- Monero node (monerod) Tor implementation (C++)
- Ethereum clients with Tor support (Go, Rust)
- Lightning Network implementations (LND, CLN, Eclair) Tor support
- Zcash node Tor integration
- Any other Rust-based blockchain nodes with Tor

**For Each, Document**:
- How do they handle SOCKS proxy configuration?
- Do they support .onion peer addresses? How stored?
- Do they bundle Tor or require system Tor?
- What libraries/approaches do they use?
- Any lessons learned or postmortems about Tor integration?

### 4. Tor Protocol & Technical Specifications

**Tor Control Protocol**:
- Read official spec: https://spec.torproject.org/control-spec/
- Focus on:
  - `ADD_ONION` command format (v3 onions specifically)
  - `DEL_ONION` command
  - `GETINFO net/listeners/socks` output format
  - Authentication methods (SAFECOOKIE, HASHEDPASSWORD)
  - Async events we should monitor (`BOOTSTRAP`, `CIRCUIT`, `STREAM`, etc.)
  - Error codes and handling

**SOCKS Extensions**:
- Read: https://spec.torproject.org/socks-extensions.html
- Understand:
  - Stream isolation via SOCKS auth (exact username/password format)
  - Tor-specific SOCKS commands
  - RESOLVE and RESOLVE_PTR for DNS over Tor
  - Error responses specific to Tor

**V3 Onion Addresses**:
- Format specification (56-character base32)
- Key generation (ed25519)
- How ephemeral vs persistent onion services differ
- Client authentication for onion services (do we need this?)

### 5. Production Deployment Challenges

**Tor Daemon Management**:
- How do production applications manage Tor daemon?
- Systemd service examples for Tor
- Docker/containerization best practices (Tor in same container or separate?)
- Windows service management for Tor
- Auto-restart and health checking strategies
- Log rotation and monitoring

**Cross-Platform Tor Installation**:
- Package names on major Linux distros (Ubuntu, Debian, CentOS, Arch)
- Homebrew formula for macOS
- Windows Tor Expert Bundle - how to distribute?
- Minimum Tor daemon version requirements
- Config file location per platform

**Security Considerations**:
- Control port security (who can access it?)
- Cookie file permissions best practices
- Should we use `ControlPortWriteToFile`?
- Network policy - should Tor be local-only or allow remote?
- Risks of bundling Tor binary (supply chain, updates)

### 6. Network Address Storage & Serialization

**.onion Address Handling**:
- How do other projects store .onion addresses in databases?
- Are there existing Rust crates for .onion address validation/parsing?
- Base32 encoding/decoding libraries for Rust
- What's the canonical way to represent .onion in memory?
  - String vs byte array vs dedicated type?
- IPv6-mapped format for .onion (does this exist/make sense?)

**Database Migration Strategies**:
- Find examples of database schema migrations adding new address types
- RocksDB key format best practices for variable-length keys
- How to maintain backward compatibility with old address format?

**Protobuf Evolution**:
- Best practices for adding oneof fields to existing protobuf schemas
- Backward/forward compatibility considerations
- How do gRPC clients handle unknown message variants?

### 7. Performance & Scalability

**Tor Network Performance**:
- Latency overhead of Tor (typical added latency in ms)
- Bandwidth limitations through Tor
- Circuit building time (how long does first connection take?)
- Connection pool implications (should we cache Tor circuits?)

**SOCKS Proxy Performance**:
- Overhead of SOCKS5 protocol
- Connection establishment latency
- Are there any async/await gotchas with SOCKS libraries?
- Connection pooling through SOCKS proxy

**Resource Usage**:
- Memory usage of Tor daemon (typical RSS)
- CPU usage during normal operation
- Disk space for Tor state
- Arti vs C Tor resource comparison

### 8. Testing & Development

**Local Tor Testing**:
- How to set up local Tor network for testing (chutney, shadow)?
- Can we mock Tor control protocol for unit tests?
- How to test .onion connectivity without real Tor network?
- Docker Compose examples for Tor + app testing

**Integration Testing**:
- How do other projects test Tor integration in CI/CD?
- Can GitHub Actions run Tor daemon?
- Cross-platform testing strategies

### 9. Edge Cases & Error Handling

**Tor Failures**:
- What happens when Tor daemon crashes during operation?
- How to handle Tor bootstrap failures?
- Circuit failures (when Tor can't build circuit to destination)
- What if control port becomes unavailable mid-operation?
- Tor daemon restart scenarios

**Network Transitions**:
- Starting in clearnet mode, adding Tor later
- Starting in Tor mode, Tor becomes unavailable
- Mixed mode (some peers via Tor, some clearnet)
- Graceful degradation strategies

**Address Validation**:
- What if peer advertises invalid .onion address?
- What if .onion service is unreachable?
- How to detect and ban malicious .onion addresses?
- Rate limiting connection attempts to .onion peers

### 10. Compliance & Privacy

**Privacy Guarantees**:
- What privacy guarantees does Tor actually provide for P2P networking?
- Known attacks on Tor hidden services
- Correlation attacks between clearnet and Tor identities
- Should we enforce Tor-only mode by default for privacy?

**Legal/Compliance**:
- Any legal considerations for bundling Tor?
- Tor Project's recommendations for integrating Tor
- Attribution/licensing requirements

## Research Methodology

For each area above:

1. **Search Patterns**:
   - Official documentation (Tor Project, Arti, crate docs)
   - GitHub repos (code examples, issues, discussions)
   - Blog posts & technical articles (2023-2025 only)
   - Stack Overflow & Reddit (recent questions/answers)
   - Academic papers if relevant

2. **Validation Criteria**:
   - Primary sources preferred (official docs, source code)
   - Check publication dates (prefer Oct 2024 - Oct 2025)
   - Verify information across multiple sources
   - Note any conflicting information

3. **Documentation Requirements**:
   - Cite all sources with URLs
   - Quote relevant code snippets
   - Note version numbers and dates
   - Flag uncertainties or conflicting info

## Deliverables

Create a comprehensive research report with:

1. **Executive Summary** (1-2 pages)
   - Key findings
   - Critical blockers or risks identified
   - Recommendation: proceed / modify approach / halt

2. **Detailed Findings** (organized by research area above)
   - Each finding with citations
   - Code examples where applicable
   - Comparisons between approaches

3. **Gap Analysis**
   - What did initial research miss?
   - What assumptions were incorrect?
   - What new information changes the approach?

4. **Updated Implementation Recommendations**
   - Library choices (final recommendations)
   - Architecture modifications based on findings
   - Risk mitigation strategies

5. **Open Questions**
   - What still needs investigation?
   - What requires prototyping to answer?
   - What decisions need user input?

## Success Criteria

Your research is complete when:

- [ ] All Arti onion service questions answered definitively
- [ ] SOCKS library choice validated with working examples
- [ ] Bitcoin Core implementation understood at code level
- [ ] At least 3 other blockchain Tor integrations analyzed
- [ ] All edge cases and failure modes documented
- [ ] Cross-platform deployment path is clear
- [ ] Testing strategy is defined
- [ ] No major unknowns remaining before implementation

## Time Estimate

This research should take **4-8 hours** of focused investigation. Prioritize CRITICAL items (Arti status, SOCKS libraries, Bitcoin Core deep dive) first.

## Questions?

If you need clarification on any research area or find something that requires immediate attention, flag it immediately.

---

**Good luck! We're counting on you to validate our approach before we commit to implementation. Be thorough!** 🔍
