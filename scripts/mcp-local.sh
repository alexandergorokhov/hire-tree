#!/usr/bin/env bash
# MCP playground for any MCP client (a judge's Claude, Codex, ...): a local
# NEAR node, the contracts, funded demo accounts, the scripted test model.
# No keys, no network. Run `make mcp-setup` once first (builds everything).
cd "$(dirname "$0")/.." || exit 1
bin="${CARGO_TARGET_DIR:-agents/target}/debug/hiretree"
if [ ! -x "$bin" ] || [ ! -f out/hire_tree_pool.wasm ]; then
  echo "hire-tree: not built yet; run 'make mcp-setup' in $(pwd) once, then restart the MCP server" >&2
  exit 1
fi
# Keep a shim the caller already loaded (make sandbox does): under it the
# ulimit check below passes, but only because of the shim.
pre="${LD_PRELOAD:-}"
if [ -z "$pre" ] && ! ulimit -n 65535 2>/dev/null; then
  mkdir -p agents/target
  if [ ! -f agents/target/fdshim.so ] && ! cc -shared -fPIC -O2 -o agents/target/fdshim.so agents/fdshim/fdshim.c -ldl; then
    echo "hire-tree: the open-file limit is too low for the local node and no C compiler (cc) was found to build the shim; install gcc or raise 'ulimit -n' to 65535, then restart the MCP server" >&2
    exit 1
  fi
  pre="$PWD/agents/target/fdshim.so"
fi
# JSON-RPC goes to fd 3 (the client's stdout); fd 1 goes to stderr so the
# local node's output can never break the stream.
exec 3>&1 1>&2
export LD_PRELOAD="$pre" HT_MCP_OUT_FD=3
if ! command -v setsid >/dev/null; then
  exec "$bin" mcp --local
fi
# Run the server in its own process group, so that when the client stops us
# with SIGTERM (most do) the local node goes down with it instead of being
# left running. Closing stdin also shuts both down.
setsid "$bin" mcp --local <&0 &
pid=$!
trap 'kill -TERM -- "-$pid" 2>/dev/null' TERM INT HUP
wait "$pid"
code=$?
wait "$pid" 2>/dev/null
exit "$code"
