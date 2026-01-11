---
id: muninn-privacy-first-recursive
level: vision
title: "Muninn: Privacy-First Recursive Context Gateway for Agentic Coding"
short_code: "PROJEC-V-0001"
created_at: 2026-01-07T14:45:52.189239+00:00
updated_at: 2026-01-07T16:21:00.714736+00:00
archived: false

tags:
  - "#vision"
  - "#phase/published"


exit_criteria_met: false
strategy_id: NULL
initiative_id: NULL
---

# Muninn: Privacy-First Recursive Context Gateway for Agentic Coding

*Named for Odin's raven of Memory - representing intelligent recall, wisdom, and the hacker ethos.*

## Purpose

Muninn exists to solve the fundamental tension between AI coding assistants and real-world software development: **LLMs cannot effectively reason about codebases they cannot fully see, yet privacy, cost, and context limits prevent them from seeing everything.**

Muninn enables privacy-conscious developers and teams to use AI coding agents on large codebases without sacrificing code privacy, accumulating unsustainable token costs, or suffering from the amnesia that plagues current AI tooling.

## The Problem

Developers using AI coding assistants face a trilemma:

1. **Context Limits Kill Productivity** - LLMs hallucinate and produce wrong answers when they can't see the full picture. "Please re-read the whole codebase" doesn't scale to millions of tokens.

2. **Privacy and Data Sovereignty** - Many developers and organizations cannot send proprietary code to cloud AI providers. Current solutions force a choice between capability and confidentiality.

3. **Persistent Amnesia** - Every AI session starts fresh. The assistant re-derives architectural decisions, re-discovers conventions, and re-learns the codebase structure repeatedly. Institutional knowledge doesn't accumulate.

## The Solution

Muninn is a **recursive context gateway** that sits between AI coding agents (like Claude Code) and LLM backends. It transforms the relationship between context and computation:

**Context tokens become compute, not storage.**

Instead of stuffing millions of tokens into a prompt, Muninn uses Recursive Language Model (RLM) techniques to let the LLM programmatically explore, decompose, and selectively retrieve only the context that matters for each query.

Combined with **persistent, repo-local memory**, Muninn remembers what it learns - architectural decisions, code relationships, past explorations - so the AI gets smarter about your codebase over time, not just within a session.

## Target Users

Privacy-conscious developers and teams who:
- Work on codebases too large for naive context windows
- Cannot or prefer not to send code to cloud AI providers
- Want AI that learns and remembers their codebase conventions
- Value open source, self-hosted tooling they control

## Core Capabilities

### 1. Intelligent Context Selection
RLM-style recursive exploration that beats naive RAG or embedding-based retrieval. The LLM actively investigates the codebase using tools (grep, read, list, tree) and recursively refines its understanding until it finds exactly what's relevant.

### 2. Persistent Memory
Vector storage (sqlite-vec) and knowledge graph (graphqlite) maintain semantic memory across sessions:
- "Last time you asked about auth, these files were relevant"
- "This module depends on these architectural decisions"
- "The team convention for error handling is..."

### 3. Invisible Integration
Drop-in OpenAI-compatible proxy. Claude Code (or any agent) doesn't know Muninn exists - it just sees "a model that suddenly understands the whole project."

### 4. Repo-Local Storage
Memory lives inside the repository (`.muninn/`):
- Version controlled alongside code
- Shared via normal git workflows
- Rebuildable from source if needed
- Portable - move the repo, memory comes with it

### 5. Metis Integration
Loosely coupled integration with Metis work management:
- Context awareness of current tasks and initiatives
- Memory of ADRs and architectural decisions
- Understanding of project structure and priorities

## Current State

- RLM concepts proven in academic research (arXiv:2512.24601) and rlmgw reference implementation
- Local LLM serving mature (vLLM, ollama)
- Claude Code and similar agents widely adopted but struggle with large codebases
- No production-ready, privacy-first solution combines intelligent context selection with persistent memory

## Future State

Developers run `muninn serve`, point Claude Code at it, and experience:
- AI that understands million-line codebases without hallucination
- Memory that accumulates across sessions and team members
- Complete privacy - all processing local, code never leaves the machine
- Seamless integration - no workflow changes required

## Success Criteria

**6-12 Month Horizon:**
- Community traction: meaningful GitHub stars, active contributors
- Adoption stories: blog posts, conference talks from users
- Reliability: works consistently on diverse real-world codebases
- Documentation: clear guides for setup and contribution

## Principles

1. **Privacy is non-negotiable** - Code never leaves the user's control. No telemetry, no cloud dependencies, no exceptions.

2. **Invisible by default** - Agents shouldn't need to know Muninn exists. Drop-in compatibility over custom integrations.

3. **Memory is a repo artifact** - Persistent state belongs in the repository, version controlled and shareable, not in external infrastructure.

4. **Backend-agnostic** - Work with any OpenAI-compatible LLM backend. Local-first but not local-only.

5. **Computation over storage** - Prefer intelligent exploration to brute-force context stuffing. Quality of context matters more than quantity.

6. **Rebuildable over opaque** - All persistent state should be regenerable from source. No black boxes.

## Non-Goals

- **Not a full IDE** - Code editing is Claude Code's job. Muninn provides context, not UI.
- **Not a cloud SaaS** - Always self-hosted. We will never offer a managed service.
- **Not a replacement for understanding** - Muninn augments developers, not replaces them. The human remains in control.

## Technical Foundation

- **Clean-room implementation** inspired by RLM concepts, not a fork of rlmgw
- **CLI-first deployment** (`muninn serve`) with Docker as secondary option
- **SQLite-based persistence** using sqlite-vec (vectors) and graphqlite (knowledge graph)
- **OpenAI-compatible API** for seamless proxy behavior