# Critical Areas Requiring Validation

This document highlights the **highest priority** areas where our initial research has gaps, uncertainties, or conflicting information. Your research engineer should focus here first.

## 🚨 PRIORITY 1: CRITICAL UNKNOWNS

### 1. Arti Onion Service Production Readiness ⚠️

**Status**: CONFLICTING INFORMATION

**What We Found**:
- ✅ Arti 1.0.0 declared "production ready" by Tor Project
- ✅ Arti 1.3.0 (Oct 2024) achieved "client feature parity"
- ✅ `tor-hsservice` crate exists
- ✅ Versions 1.4.x-1.6.x have onion service development
- ❌ BUT Feb 2024 docs say "features missing for secure, private onion service"

**The Problem**: We don't know if we can use Arti for onion services TODAY (Oct 2025) or if we must use C Tor daemon.

**Why It Matters**: This fundamentally changes our architecture:
- **If Arti works**: Pure Rust, in-process, no external dependencies
- **If Arti doesn't work**: Must manage C Tor daemon, cross-platform binaries, process lifecycle

**Research Needed**:
- [ ] Check Arti 1.6.0 documentation for current onion service status
- [ ] Look for production projects using Arti onion services (GitHub search)
- [ ] Find Arti issue tracker discussions about onion service stability
- [ ] Contact Tor Project or check their forums for guidance
- [ ] Test `tor-hsservice` crate if possible (quick prototype)

**Decision Impact**: HIGH - Affects Phase 2-5 of implementation

---

### 2. Tonic + SOCKS5 Integration Method ⚠️

**Status**: UNCERTAIN

**What We Found**:
- Identified `tokio-socks` (most popular) and `hyper-socks2` as options
- Assumption: Can use Tonic's `connect_with_connector()` with custom connector
- No confirmed working examples found

**The Problem**: We don't have a proven pattern for SOCKS + Tonic integration.

**Why It Matters**:
- This is the CORE of Phase 1 (outbound connections via Tor)
- If this doesn't work, we need a different approach
- Might require wrapping at a different layer

**Research Needed**:
- [ ] Search GitHub for "tonic socks5 proxy" - find working code
- [ ] Search for "tokio-socks tower service" - Tower is under Tonic
- [ ] Check if hyper-socks2 is actually compatible with Hyper 1.x
- [ ] Look for gRPC + SOCKS examples in any language (pattern learning)
- [ ] Check Tonic docs/examples for custom connector usage

**Validation Method**:
- Must find at least ONE working example of Tonic + SOCKS
- OR prototype this ourselves to prove it works

**Decision Impact**: CRITICAL - Blocks Phase 1 if not possible

---

### 3. tor-interface Crate Reliability 🤔

**Status**: LIMITED VALIDATION

**What We Found**:
- Version 0.6.0 released Oct 2025 (recent)
- Has `LegacyTorClient` for C Tor daemon control
- Looks feature-complete on paper

**The Problem**:
- We found no production usage examples
- Unknown cross-platform reliability
- Unknown if it handles all edge cases

**Why It Matters**:
- This is our primary interface to Tor daemon (Phase 2-3)
- If it's buggy/incomplete, we need alternative

**Research Needed**:
- [ ] Find who created/maintains `tor-interface`
- [ ] Look for production projects using it
- [ ] Check issue tracker for bugs
- [ ] Look for alternatives (other Tor control protocol crates?)
- [ ] Check last commit date and maintenance status

**Decision Impact**: MEDIUM - Can fall back to direct control protocol if needed

---

## 🔍 PRIORITY 2: HIGH-VALUE VALIDATIONS

### 4. Bitcoin Core Implementation Details

**Status**: HIGH-LEVEL UNDERSTANDING ONLY

**What We Need**:
- Actual Bitcoin Core source code review
- Exact SOCKS connector implementation
- Stream isolation format (SOCKS username pattern)
- .onion address storage format
- Error handling patterns

**Why It Matters**: Bitcoin Core is the gold standard - we should copy what works.

**Research Needed**:
- [ ] Read Bitcoin Core `net.cpp`, `torcontrol.cpp` source files
- [ ] Document their SOCKS integration approach
- [ ] Find their .onion address type/storage
- [ ] Note any gotchas in comments/commit messages

---

### 5. .onion Address Best Practices

**Status**: ASSUMPTION-BASED

**Our Approach**: Store as string in enum variant

**Research Needed**:
- [ ] How do other Rust projects handle .onion addresses?
- [ ] Is there a standard Rust crate for .onion validation/parsing?
- [ ] Should we use String, [u8; 56], or dedicated type?
- [ ] Base32 encoding considerations?

---

### 6. Stream Isolation Implementation

**Status**: THEORETICAL

**Our Plan**: Random username per connection in SOCKS auth

**Research Needed**:
- [ ] Is this the correct approach?
- [ ] What format should username be? (UUID, random number, counter?)
- [ ] Do we need password field or just username?
- [ ] Are there Tor-specific requirements?
- [ ] Performance implications of creating new circuit per connection?

---

## 🧪 PRIORITY 3: VALIDATION VIA EXAMPLES

### 7. Find Production Examples

**Goal**: Find at least 3 production projects with Tor integration in Rust or similar stack

**Why**: Real-world examples reveal edge cases and best practices we missed

**Search For**:
- Rust blockchain nodes with Tor (any cryptocurrency)
- Rust P2P applications using Tor
- Go/Rust projects with SOCKS + gRPC
- Lightning Network implementations (LND, CLN have Tor)

**Document**:
- What libraries they use
- How they structure the integration
- What problems they encountered (issues, PRs, commit messages)
- Any performance/security gotchas

---

## 📋 VALIDATION CHECKLIST

Before proceeding to implementation, we must have:

**Architecture Decisions**:
- [ ] **CONFIRMED**: Arti onion services work OR we're using C Tor daemon
- [ ] **PROVEN**: Tonic + SOCKS integration pattern (with working example or prototype)
- [ ] **VALIDATED**: tor-interface reliability OR identified alternative

**Technical Approach**:
- [ ] **VERIFIED**: Bitcoin Core approach reviewed and understood
- [ ] **CONFIRMED**: .onion address handling pattern validated
- [ ] **TESTED**: Stream isolation approach confirmed correct

**Risk Assessment**:
- [ ] **DOCUMENTED**: All known edge cases and failure modes
- [ ] **PLANNED**: Cross-platform deployment strategy validated
- [ ] **IDENTIFIED**: Testing approach defined

**No Blockers**:
- [ ] **VERIFIED**: No technical blockers to Phase 1 (SOCKS proxy)
- [ ] **VERIFIED**: No technical blockers to Phase 2 (onion services)
- [ ] **VERIFIED**: No technical blockers to Phase 3 (.onion addresses)

---

## ⏱️ Estimated Research Time

**PRIORITY 1 (Critical)**: 2-3 hours
- Arti validation: 1 hour
- Tonic+SOCKS: 1 hour
- tor-interface: 30 min

**PRIORITY 2 (High-Value)**: 2-3 hours
- Bitcoin Core review: 1.5 hours
- Other examples: 1 hour
- Stream isolation: 30 min

**PRIORITY 3 (Due Diligence)**: 2-3 hours
- Platform research: 1 hour
- Testing strategy: 1 hour
- Edge cases: 1 hour

**Total**: 6-9 hours for thorough validation

---

## 🎯 Success Criteria

Research is complete when you can answer with confidence:

1. **Can we use Arti for onion services today?** (Yes/No + evidence)
2. **Does Tonic + SOCKS work?** (Yes + example OR No + alternative)
3. **Is our architecture sound?** (Validated by real examples)
4. **Are there any blockers?** (None OR identified with mitigation)

If you can't answer these with high confidence, **DO NOT PROCEED TO IMPLEMENTATION**.

---

## 📞 Escalation

If you discover:
- A fundamental blocker (e.g., "Tonic + SOCKS is impossible")
- Major architectural flaw in our plan
- Security vulnerability we missed
- Better approach that invalidates current design

**STOP and escalate immediately.** Don't wait for full research completion.
