"""Translate existing GitHub fixtures into aliased status-batch responses."""


def history_for_pr(details):
    return {"ref": {"target": {"history": {"nodes": [
        node["commit"] for node in details.get("commits", {}).get("nodes", [])
    ]}}}}


def status_response(args, details, history, branch_history=None):
    fields = dict(
        argument.split("=", 1)
        for index, argument in enumerate(args)
        if index > 0 and args[index - 1] in ("-f", "-F")
    )
    data = {}
    indices = sorted(int(key.removeprefix("owner")) for key in fields if key.startswith("owner"))
    for index in indices:
        if f"pr{index}" in fields:
            value = dict(details(int(fields[f"pr{index}"])))
            if "statusCheckRollup" in value:
                checks = value.pop("statusCheckRollup")
                value["commits"] = {"nodes": [{"commit": {
                    "oid": "a" * 40,
                    "statusCheckRollup": {"contexts": {
                        "nodes": checks,
                        "pageInfo": {"hasNextPage": False, "endCursor": None},
                    }} if checks is not None else None,
                }}]}
            data[f"target{index}"] = {"pullRequest": value}
            if f"ciOwner{index}" in fields:
                data[f"ci{index}"] = (
                    branch_history["data"]["repository"] if branch_history is not None
                    else history_for_pr(value)
                )
        else:
            data[f"target{index}"] = history["data"]["repository"]
    return {"data": data}
