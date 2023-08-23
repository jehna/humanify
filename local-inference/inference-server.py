import zmq
import json
from rename import rename
from define import define, desc_to_name

context = zmq.Context()
socket = context.socket(zmq.REP)
socket.bind("ipc:///tmp/humanify-local-inference-server.ipc")

print("Server started")

while True:
    # JSON parse the message
    message = json.loads(socket.recv())

    match message["type"]:
        case "rename":
            before = message['before']
            after = message['after']
            var_name = message['varname']
            description = message['description']
            filename = message['filename']
            renamed = rename(before, after, var_name, filename)
            socket.send_string(json.dumps({"type": "renamed", "renamed": renamed}))
        case "define":
            code = message['code']
            description = define(code)
            filename = desc_to_name(description)
            socket.send_string(json.dumps({"type": "defined", "description": description, "filename": filename}))
