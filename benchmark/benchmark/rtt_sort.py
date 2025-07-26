import sys
from ping3 import ping
import time
import json

TIMEOUT = 1

def ping_host(host, count):
    rtts = []
    for _ in range(count):
        try:
            rtt = ping(host, timeout=TIMEOUT)
            if rtt is not None:
                rtts.append(rtt * 1000)  # convert to ms
        except Exception:
            pass
        time.sleep(0.2)
    if rtts:
        avg_rtt = sum(rtts) / len(rtts)
    else:
        avg_rtt = float('inf')
    return avg_rtt


def sort_nodes_by_rtt(host_list, count=5):
    results = []
    for host  in host_list:
        avg_rtt = ping_host(host, count)
        if avg_rtt == float('inf'):
            print(f"{host:35} - Unreachable")
        results.append((host, avg_rtt))

    results.sort(key=lambda x: x[1], reverse=True)
    return [x[0] for x in results]


def main():
    if len(sys.argv) > 1:
        json_string = sys.argv[1]
        try:
            host_list = json.loads(json_string)
            assert all(isinstance(x, str) for x in host_list)
            print(json.dumps(sort_nodes_by_rtt(host_list)), end="")
        except json.JSONDecodeError:
            print("Error: Invalid JSON format.")
        except AssertionError:
            print("Invalid input")
    else:
        print("No JSON input provided.")

if __name__ == "__main__":
    main()