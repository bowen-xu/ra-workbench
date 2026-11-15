// SPDX-License-Identifier: AGPL-3.0-or-later
import { createServer } from "node:http";
import { createPreviewHandler } from "./preview-server.mjs";

const port = Number(process.env.RA_CONNECTOR_PREVIEW_PORT ?? "5187");
if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error("Invalid preview port");
const server = createServer(await createPreviewHandler());
server.on("error", error => { console.error(error.code === "EADDRINUSE" ? `Preview port ${port} is occupied; choose RA_CONNECTOR_PREVIEW_PORT.` : "Unable to start preview server"); process.exitCode = 1; });
server.listen(port, "127.0.0.1", () => { console.log(`Chrome Connector preview: http://127.0.0.1:${port}/`); });
for (const signal of ["SIGINT", "SIGTERM"]) process.once(signal, () => { server.close(); server.closeAllConnections(); });
