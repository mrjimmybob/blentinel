"""A stand-in for the parts of GitHub's API the deployment agent uses, for
its end-to-end test (deploy-test.sh, D-107). Never used against GitHub.

    python3 -I fake-github.py <folder> <port>

<folder>/state.json is read at every request, so the test changes what
"GitHub" holds between runs of the agent:

    {"token": "...", "runs": [{"id", "run_number", "head_sha", ...}],
     "artifacts": {"<run id>": [{"id", "name", "file"}]},
     "faults": {"create": 0, "success": 0}}

Deployments and their statuses are kept in <folder>/deployments.json, and
every request is logged to <folder>/requests.jsonl (method, path, whether
it carried the token), for the test to check. A fault count above zero
makes the next such request fail with 500, once per count.
"""

import json
import pathlib
import re
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

FOLDER = pathlib.Path(sys.argv[1])
PORT = int(sys.argv[2])
LOCK = threading.Lock()


def load(name, default):
    path = FOLDER / name
    return json.loads(path.read_text()) if path.exists() else default


def save(name, value):
    (FOLDER / name).write_text(json.dumps(value, indent=1))


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, code, body=None, headers=()):
        data = b"" if body is None else json.dumps(body).encode()
        self.send_response(code)
        for name, value in headers:
            self.send_header(name, value)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def handle_any(self, method):
        with LOCK:
            state = load("state.json", {})
            url = urlparse(self.path)
            auth = self.headers.get("Authorization")
            with open(FOLDER / "requests.jsonl", "a") as log:
                log.write(json.dumps({"method": method, "path": url.path, "query": url.query,
                                      "authorized": auth is not None}) + "\n")
            body = None
            if method == "POST":
                body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")

            # The storage an artifact's download is redirected to: no token.
            match = re.fullmatch(r"/blob/(\d+)", url.path)
            if match and method == "GET":
                for artifacts in state["artifacts"].values():
                    for artifact in artifacts:
                        if str(artifact["id"]) == match.group(1):
                            data = pathlib.Path(artifact["file"]).read_bytes()
                            self.send_response(200)
                            self.send_header("Content-Type", "application/zip")
                            self.send_header("Content-Length", str(len(data)))
                            self.end_headers()
                            self.wfile.write(data)
                            return
                return self.reply(404, {"message": "Not Found"})

            if auth != f"Bearer {state['token']}":
                return self.reply(401, {"message": "Bad credentials"})
            repo = "/repos/" + state["repository"]
            path = url.path
            query = parse_qs(url.query)

            if method == "GET" and path == f"{repo}/actions/workflows/build.yml/runs":
                runs = [dict(r, head_branch=r.get("head_branch", "main"), event=r.get("event", "push"),
                             status="completed", conclusion=r.get("conclusion", "success"))
                        for r in state["runs"]]
                return self.reply(200, {"total_count": len(runs), "workflow_runs": runs})
            match = re.fullmatch(rf"{repo}/actions/runs/(\d+)/artifacts", path)
            if method == "GET" and match:
                artifacts = [{"id": a["id"], "name": a["name"], "expired": False, "size_in_bytes": 1}
                             for a in state["artifacts"].get(match.group(1), [])]
                return self.reply(200, {"total_count": len(artifacts), "artifacts": artifacts})
            match = re.fullmatch(rf"{repo}/actions/artifacts/(\d+)/zip", path)
            if method == "GET" and match:
                return self.reply(302, headers=[("Location", f"http://127.0.0.1:{PORT}/blob/{match.group(1)}")])

            deployments = load("deployments.json", [])
            if method == "GET" and path == f"{repo}/deployments":
                found = [d for d in deployments
                         if d["sha"] == query.get("sha", [""])[0]
                         and d["environment"] == query.get("environment", [""])[0]]
                return self.reply(200, found)
            if method == "POST" and path == f"{repo}/deployments":
                if state["faults"].get("create", 0) > 0:
                    state["faults"]["create"] -= 1
                    save("state.json", state)
                    return self.reply(500, {"message": "injected fault"})
                deployment = {"id": 9000 + len(deployments), "sha": body["ref"], "ref": body["ref"],
                              "environment": body["environment"], "payload": body["payload"],
                              "description": body["description"], "statuses": []}
                deployments.append(deployment)
                save("deployments.json", deployments)
                return self.reply(201, deployment)
            match = re.fullmatch(rf"{repo}/deployments/(\d+)/statuses", path)
            if method == "POST" and match:
                if body["state"] == "success" and state["faults"].get("success", 0) > 0:
                    state["faults"]["success"] -= 1
                    save("state.json", state)
                    return self.reply(500, {"message": "injected fault"})
                for deployment in deployments:
                    if str(deployment["id"]) == match.group(1):
                        deployment["statuses"].append({"state": body["state"], "description": body["description"]})
                        save("deployments.json", deployments)
                        return self.reply(201, {"state": body["state"]})
                return self.reply(404, {"message": "Not Found"})
            return self.reply(404, {"message": "Not Found"})

    def do_GET(self):
        self.handle_any("GET")

    def do_POST(self):
        self.handle_any("POST")


ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
