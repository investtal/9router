# Code Intelligence — graphify

This project uses **graphify** for code intelligence. The knowledge graph lives in `graphify-out/graph.json` (5220 nodes, 10948 edges, 377 communities).

## Always Do

- **MUST check blast radius before editing symbols.** Before modifying a function, class, or method, run `graphify query "what calls <symbolName>"` and report affected communities to the user.
- **MUST check `graphify query` before committing** to verify changes only touch expected communities.
- When exploring unfamiliar code, run `graphify query "<concept>"` instead of grepping — returns node-grouped results ranked by relevance.
- When you need full context on a symbol — callers, callees, which community it belongs to — run `graphify explain "<symbolName>"` or `graphify path "<A>" "<B>"` for shortest path between concepts.
- After structural code changes, rebuild with `/graphify . --update` to refresh the graph incrementally.

## Never Do

- NEVER edit a function, class, or method without first checking blast radius via `graphify query`.
- NEVER ignore surprising-connection or god-node warnings the graph surfaces.
- NEVER rename symbols with find-and-replace — run `graphify query "<symbol>"` first to see all references.

## Resources

| Resource | Use for |
|----------|---------|
| `graphify-out/graph.json` | Raw graph data (GraphRAG-ready) |
| `graphify-out/GRAPH_REPORT.md` | Audit report: god nodes, surprising connections, suggested questions |
| `graphify-out/graph.html` | Interactive graph, open in browser |

## CLI

| Task | Command |
|------|---------|
| Understand architecture / "How does X work?" | `graphify query "how does X work"` (BFS) |
| Blast radius / "What breaks if I change X?" | `graphify query "what calls X"` |
| Trace a specific path | `graphify query "X" --dfs` |
| Shortest path between two concepts | `graphify path "A" "B"` |
| Plain-language explanation of a node | `graphify explain "SymbolName"` |
| Incremental rebuild after code changes | `/graphify . --update` |
| Full rebuild | `/graphify .` |
