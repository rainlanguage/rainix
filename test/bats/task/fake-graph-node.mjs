// A stand-in for a Graph node and its IPFS endpoint, enough for a real
// `graph deploy` to compile, upload and then be refused: every IPFS add and
// pin succeeds, and every JSON-RPC call (the subgraph_deploy request) answers
// with the error Ormi gives for a build it already hosts.
// Usage: node fake-graph-node.mjs <port-file>
// Listens on a free loopback port and writes that port to <port-file>.
import { createServer } from "node:http";
import { writeFileSync } from "node:fs";

const CID = "QmT6ZDiVeJkAftawHYUStSJ1t39JibndWKKW89ThY8AXoX";

const server = createServer((req, res) => {
  let body = "";
  req.on("data", (chunk) => (body += chunk));
  req.on("end", () => {
    const url = new URL(req.url, "http://localhost");
    res.setHeader("content-type", "application/json");
    if (url.pathname.endsWith("/pin/add")) {
      res.end(JSON.stringify({ Pins: [CID] }));
    } else if (url.pathname.endsWith("/add")) {
      res.end(JSON.stringify({ Name: "file", Hash: CID, Size: "1" }) + "\n");
    } else {
      const { id } = JSON.parse(body);
      res.end(
        JSON.stringify({
          jsonrpc: "2.0",
          id,
          error: {
            code: -32000,
            message: "E017: You've already deployed this subgraph",
          },
        }),
      );
    }
  });
});

server.listen(0, "127.0.0.1", () => {
  writeFileSync(process.argv[2], String(server.address().port));
});
