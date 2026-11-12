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
//! - [`swarm`]: `hiretree swarm`, many strategy agents over many jobs.
//! - [`verifier`]: NEAR Intents: the official Verifier wasm, pay landing there, swaps.
//! - [`judge`]: `hiretree mcp --local`, a playground on a local node for any MCP client.
//! - [`rubric`]: `hiretree rubric-check`, the live run graded again under judge rubric 2.

pub mod agents;
pub mod chain;
pub mod intent;
pub mod judge;
pub mod llm;
pub mod mcp;
pub mod rubric;
pub mod scene;
pub mod swarm;
pub mod tools;
pub mod verifier;
