#!/usr/bin/env bash
# Build the static site for GitHub Pages into out/pages: the swarm page is
# the landing page (index.html), the 23-scene story moves to story.html.
# Plain files only, no build step and no workflow. To publish (owner):
#   git worktree add ../pages gh-pages 2>/dev/null || git worktree add -b gh-pages ../pages
#   cp -r out/pages/. ../pages/ && (cd ../pages && git add -A && git commit -m site && git push -u origin gh-pages)
# then Settings > Pages > Deploy from branch gh-pages, folder /.
set -euo pipefail
cd "$(dirname "$0")/.."
test -s web/scene.json || { echo "web/scene.json missing: run make demo first" >&2; exit 1; }
rm -rf out/pages && mkdir -p out/pages
cp web/*.json out/pages/
sed 's#href="index.html"#href="story.html"#' web/swarm.html > out/pages/index.html
cp web/index.html out/pages/story.html
cp web/market.html out/pages/market.html
cp -r web/brand out/pages/brand
touch out/pages/.nojekyll
ls out/pages
