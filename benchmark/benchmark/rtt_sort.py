import sys
from ping3 import ping
import time

# List of hostnames or IP addresses of nodes
hosts = [
    "us-east1.example.com",
    "europe-west1.example.com",
    "asia-northeast1.example.com",
    "us-west1.example.com",
    "localhost"
]

def ping_host(host, count=5, timeout=1):
    rtts = []
    for _ in range(count):
        try:
            rtt = ping(host, timeout=timeout)
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

def main():
    print("Pinging nodes...\n")
    results = []
    for host in hosts:
        avg_rtt = ping_host(host)
        if avg_rtt == float('inf'):
            print(f"{host:35} - Unreachable")
        else:
            print(f"{host:35} - Avg RTT: {avg_rtt:.2f} ms")
        results.append((host, avg_rtt))

    print("\nSorted by farthest (highest RTT) first:\n")
    results.sort(key=lambda x: x[1], reverse=True)
    for host, rtt in results:
        label = f"{rtt:.2f} ms" if rtt != float('inf') else "Unreachable"
        print(f"{host:35} - {label}")

if __name__ == "__main__":
    main()