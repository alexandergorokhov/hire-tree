//! Hire Tree agents: a lead agent that plans, picks workers by their
//! on-chain track record, hires, reviews and pays early or disputes; worker
//! agents that deliver NEAR AI output with its signed receipt; a keeper that
//! answers the contract's own check question with a signed NEAR AI answer.
//!
//! - [`chain`]: the network (local near-sandbox or testnet).
//! - [`llm`]: NEAR AI Cloud, record/replay cassettes, and a test mock.
//! - [`tools`]: the tool surface, shared by the agents and the MCP server.
//! - [`agents`]: the agents and the two-job story.
//! - [`scene`]: writes `out/scene.json` for the web page.
//! - [`mcp`]: a stdio MCP server exposing [`tools`].

pub mod agents;
pub mod chain;
pub mod llm;
pub mod mcp;
pub mod scene;
pub mod tools;
