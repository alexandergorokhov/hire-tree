#!/usr/bin/env bash
# MCP playground for any MCP client (a judge's Claude, Codex, ...): a local
# NEAR node, the contracts, funded demo accounts, the scripted test model.
# No keys, no network. Run `make mcp-setup` once first (builds everything).
cd "$(dirname "$0")/.." || exit 1
bin=agents/target/debug/hiretree
if [ ! -x "$bin" ] || [ ! -f out/hire_tree_pool.wasm ]; then
  echo "hire-tree: not built yet; run 'make mcp-setup' in $(pwd) once, then restart the MCP server" >&2
  exit 1
fi
# Keep a shim the caller already loaded (make sandbox does): under it the
# ulimit check below passes, but only because of the shim.
pre="${LD_PRELOAD:-}"
if [ -z "$pre" ] && ! ulimit -n 65535 2>/dev/null; then
  [ -f agents/target/fdshim.so ] || cc -shared -fPIC -O2 -o agents/target/fdshim.so agents/fdshim/fdshim.c -ldl
  pre="$PWD/agents/target/fdshim.so"
fi
# JSON-RPC goes to fd 3 (the client's stdout); fd 1 goes to stderr so the
# local node's output can never break the stream.
exec 3>&1 1>&2
LD_PRELOAD="$pre" HT_MCP_OUT_FD=3 exec "$bin" mcp --local
