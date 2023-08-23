import zmq from "zeromq";
import { spawn } from "child_process";

const sock = zmq.socket("req");

sock.connect("ipc:///tmp/humanify-local-inference-server.ipc");

export function send<Recv extends {}>(message: Object) {
  sock.send(JSON.stringify(message));

  while (true) {
    // Babel only supports synchronous visitors, so we can't use async/await here.
    const reply = (sock as any).read();
    if (reply) {
      return JSON.parse(reply.toString()) as Recv;
    }
  }
}

export function createServer() {
  spawn("python", ["local-inference/inference-server.py"], {
    stdio: "inherit",
  });
}
