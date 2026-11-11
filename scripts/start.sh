#!/usr/bin/env bash
# One command to see Hire Tree run locally: build the contracts and the
# agent binary, run the agent story on a local NEAR node (mock model, fake
# money, free), then serve the pages in the background.
#
#   make start        # or: scripts/start.sh   (PORT=8080 by default)
#
# About 6-8 min warm (a cold first build adds several minutes). It ends with
# the URLs to open and the MCP next step.
set -euo pipefail
cd "$(dirname "$0")/.."
port="${PORT:-8080}"
mkdir -p out
echo "1/3 build contracts + agent binary (make mcp-setup)"
make --no-print-directory mcp-setup >out/start-build.log 2>&1 || { tail -20 out/start-build.log; exit 1; }
echo "2/3 the agent story on a local NEAR node (make demo, about 5 min; log: out/start-demo.log)"
make --no-print-directory demo >out/start-demo.log 2>&1 || { tail -30 out/start-demo.log; exit 1; }
grep -E "model calls|transactions" out/start-demo.log | tail -n 1 || true
echo "3/3 serve web/ on http://localhost:$port"
if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
  echo "    something already listens on port $port; set PORT=8081 to use another"
else
  nohup python3 -m http.server "$port" --directory web >out/start-web.log 2>&1 &
  echo "    server pid $! (stop it with: kill $!)"
fi
cat <<EOF

DEMO PASS: the story ran on a local node and wrote web/scene.json
Open:
  http://localhost:$port/market.html   agents ranked by record, every job's tree and judge scores
  http://localhost:$port/index.html    the story you just ran, scene by scene
  http://localhost:$port/swarm.html    the 12-round market charts
Agents (MCP): restart your MCP client in this folder; .mcp.json starts the
"hire-tree" playground. Call guide first. Allow tool calls up to 300 s.
EOF
