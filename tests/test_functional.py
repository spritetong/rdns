"""
Automated functional test suite for RDNS using FastAPI mock server.
"""

import os
import sys

if sys.platform == 'win32':
    try:
        sys.stdout.reconfigure(encoding='utf-8')  # pyright: ignore[reportAttributeAccessIssue]
        sys.stderr.reconfigure(encoding='utf-8')  # pyright: ignore[reportAttributeAccessIssue]
    except:  # noqa: E722 S110
        pass

import json
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ROOT_DIR = Path(__file__).parent.parent
MOCK_PORT = 8899
BASE_URL = f'http://127.0.0.1:{MOCK_PORT}'
CONFIG_PATH = ROOT_DIR / 'tests' / 'test_config.yaml'
STATE_FILE = ROOT_DIR / 'state.json'


def get_binary_path():
    exe_name = 'rdns.exe' if os.name == 'nt' else 'rdns'
    bin_path = ROOT_DIR / 'target' / 'debug' / exe_name
    if not bin_path.exists():
        raise FileNotFoundError(
            f"RDNS binary not found at {bin_path}. Run 'cargo build' first."
        )
    return bin_path


def http_get(path):
    url = f'{BASE_URL}{path}'
    req = urllib.request.Request(url)
    with urllib.request.urlopen(req, timeout=5) as resp:
        return resp.read().decode('utf-8')


def http_post(path, data=None):
    url = f'{BASE_URL}{path}'
    body = json.dumps(data).encode('utf-8') if data else b''
    req = urllib.request.Request(
        url, data=body, headers={'Content-Type': 'application/json'}
    )
    with urllib.request.urlopen(req, timeout=5) as resp:
        return resp.read().decode('utf-8')


def wait_for_server(max_retries=30):
    for _ in range(max_retries):
        try:
            res = http_get('/health')
            if 'ok' in res:
                return True
        except:  # noqa: E722
            time.sleep(0.2)
    return False


def reset_server():
    http_post('/api/reset')


def get_records():
    raw = http_get('/api/records')
    return json.loads(raw)


def run_tests():
    binary = get_binary_path()
    print(f'=== Starting Functional Tests using binary: {binary} ===')

    # 1. Start Mock Server
    print('-> Starting FastAPI mock server on port', MOCK_PORT)
    server_proc = subprocess.Popen(
        [sys.executable, str(ROOT_DIR / 'tests' / 'mock_server.py'), str(MOCK_PORT)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

    try:
        if not wait_for_server():
            raise RuntimeError('Failed to start FastAPI mock server')
        print('-> Mock server is healthy!')

        # Clean up any leftover state.json
        if STATE_FILE.exists():
            STATE_FILE.unlink()

        # -------------------------------------------------------------------
        # Test 1: Configuration check (--check)
        # -------------------------------------------------------------------
        print('\n[TEST 1] Testing --check mode...')
        res = subprocess.run(
            [str(binary), '--check', '-c', str(CONFIG_PATH)],
            capture_output=True,
            check=False,
            text=True,
            encoding='utf-8',
        )
        assert res.returncode == 0, f'--check failed: {res.stderr}'
        assert 'valid' in res.stdout.lower() or res.returncode == 0
        print('  [PASS] Valid config check passed')

        # Test invalid config
        invalid_config = ROOT_DIR / 'tests' / 'invalid_config.yaml'
        invalid_config.write_text(
            'global:\n  interval: 10\ntasks: []', encoding='utf-8'
        )
        res_inv = subprocess.run(
            [str(binary), '--check', '-c', str(invalid_config)],
            capture_output=True,
            check=False,
            text=True,
            encoding='utf-8',
        )
        assert res_inv.returncode != 0, 'Invalid config should fail --check'
        invalid_config.unlink()
        print('  [PASS] Invalid config check rejected as expected')

        # -------------------------------------------------------------------
        # Test 2: Dry-run mode (--dry-run)
        # -------------------------------------------------------------------
        print('\n[TEST 2] Testing --dry-run mode...')
        reset_server()
        res = subprocess.run(
            [str(binary), '--dry-run', '-c', str(CONFIG_PATH)],
            capture_output=True,
            check=False,
            text=True,
            encoding='utf-8',
        )
        assert res.returncode == 0, f'--dry-run failed: {res.stderr}'
        assert '[DRY-RUN PREVIEW: mock-dualstack]' in res.stdout, (
            'Preview header missing'
        )
        assert 'mocksecret' not in res.stdout, 'Secret token should be masked'

        records = get_records()
        write_requests = [
            r
            for r in records
            if r['path'] in ['/nic/update', '/api/v1/ddns/update', '/api/v1/notify']
        ]
        assert len(write_requests) == 0, (
            f'Dry-run should not send write requests, found: {write_requests}'
        )
        print('  [PASS] Dry-run produced preview without sending any write requests')

        # -------------------------------------------------------------------
        # Test 3: Single run mode (--once)
        # -------------------------------------------------------------------
        print('\n[TEST 3] Testing --once live update mode...')
        reset_server()
        res = subprocess.run(
            [str(binary), '--once', '-c', str(CONFIG_PATH)],
            capture_output=True,
            check=False,
            text=True,
            encoding='utf-8',
        )
        assert res.returncode == 0, f'--once failed: {res.stderr}'

        records = get_records()
        nic_updates = [r for r in records if r['path'] == '/nic/update']
        webhook_updates = [r for r in records if r['path'] == '/api/v1/ddns/update']
        notifications = [r for r in records if r['path'] == '/api/v1/notify']

        assert len(nic_updates) == 1, f'Expected 1 /nic/update, got {len(nic_updates)}'
        assert 'myip=203.0.113.195' in nic_updates[0]['query']
        assert (
            'myipv6=2001%3Adb8%3A%3A1234%3A5678' in nic_updates[0]['query']
            or '2001:db8::1234:5678' in nic_updates[0]['query']
        )

        assert len(webhook_updates) == 1, (
            f'Expected 1 /api/v1/ddns/update, got {len(webhook_updates)}'
        )
        body_json = json.loads(webhook_updates[0]['body'])
        assert body_json['ip'] == '2001:db8::1234:5678'
        assert body_json['domain'] == 'v6.example.com'
        auth_hdr = webhook_updates[0]['headers'].get('authorization', '')
        assert 'Bearer tok123' in auth_hdr, (
            f'Expected Authorization header with env expansion, got: {auth_hdr}'
        )

        assert len(notifications) >= 1, (
            f'Expected notification webhook, got {len(notifications)}'
        )

        assert STATE_FILE.exists(), 'state.json should be created'
        state_data = json.loads(STATE_FILE.read_text(encoding='utf-8'))
        assert 'mock-dualstack' in state_data
        assert state_data['mock-dualstack']['ipv4'] == '203.0.113.195'
        assert state_data['mock-dualstack']['ipv6'] == '2001:db8::1234:5678'
        print(
            '  [PASS] Single run live update, templating, and atomic state write succeeded'
        )

        # -------------------------------------------------------------------
        # Test 4: Idempotency detection
        # -------------------------------------------------------------------
        print('\n[TEST 4] Testing IP idempotency (no IP change)...')
        reset_server()
        res = subprocess.run(
            [str(binary), '--once', '-c', str(CONFIG_PATH)],
            capture_output=True,
            check=False,
            text=True,
            encoding='utf-8',
        )
        assert res.returncode == 0

        records = get_records()
        write_requests = [
            r for r in records if r['path'] in ['/nic/update', '/api/v1/ddns/update']
        ]
        assert len(write_requests) == 0, (
            f'Unchanged IP should not trigger updates, got: {write_requests}'
        )
        print('  [PASS] Idempotency verified: redundant HTTP writes prevented')

        # -------------------------------------------------------------------
        # Test 5: Worker threads option (-t / --worker-threads)
        # -------------------------------------------------------------------
        print('\n[TEST 5] Testing custom worker threads configuration...')
        res_t1 = subprocess.run(
            [str(binary), '--once', '-c', str(CONFIG_PATH), '-t', '1'],
            capture_output=True,
            check=False,
            text=True,
            encoding='utf-8',
        )
        assert res_t1.returncode == 0
        res_t4 = subprocess.run(
            [str(binary), '--once', '-c', str(CONFIG_PATH), '--worker-threads', '4'],
            capture_output=True,
            check=False,
            text=True,
            encoding='utf-8',
        )
        assert res_t4.returncode == 0
        print('  [PASS] Single-thread (t=1) and multi-thread (t=4) runs succeeded')

        # -------------------------------------------------------------------
        # Test 6: Daemon mode and Graceful Shutdown
        # -------------------------------------------------------------------
        print('\n[TEST 6] Testing daemon mode and graceful shutdown...')
        daemon_proc = subprocess.Popen(
            [str(binary), '--daemon', '-c', str(CONFIG_PATH)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding='utf-8',
        )
        time.sleep(1.5)
        # Terminate daemon
        daemon_proc.terminate()
        try:
            daemon_proc.wait(timeout=5)
            print('  [PASS] Daemon exited cleanly within shutdown timeout')
        except subprocess.TimeoutExpired:
            daemon_proc.kill()
            raise AssertionError('Daemon failed to shut down within timeout!')

        # -------------------------------------------------------------------
        # Test 7: Testing --no-state and --write-state options
        # -------------------------------------------------------------------
        print('\n[TEST 7] Testing --no-state and --write-state false options...')
        if STATE_FILE.exists():
            STATE_FILE.unlink()

        # Run with --no-state
        res_no_state = subprocess.run(
            [str(binary), '--once', '-c', str(CONFIG_PATH), '--no-state'],
            capture_output=True,
            check=False,
            text=True,
            encoding='utf-8',
        )
        assert res_no_state.returncode == 0, f'--no-state run failed: {res_no_state.stderr}'
        assert not STATE_FILE.exists(), 'state.json must not exist when --no-state is specified'
        print('  [PASS] --no-state ran successfully without creating state file')

        # Run with --write-state false
        res_write_false = subprocess.run(
            [str(binary), '--once', '-c', str(CONFIG_PATH), '--write-state', 'false'],
            capture_output=True,
            check=False,
            text=True,
            encoding='utf-8',
        )
        assert res_write_false.returncode == 0, f'--write-state false run failed: {res_write_false.stderr}'
        assert not STATE_FILE.exists(), 'state.json must not exist when --write-state false is specified'
        print('  [PASS] --write-state false ran successfully without creating state file')

        # -------------------------------------------------------------------
        # Test 8: Testing --log-level options
        # -------------------------------------------------------------------
        print('\n[TEST 8] Testing --log-level options...')
        res_log_off = subprocess.run(
            [str(binary), '--dry-run', '-c', str(CONFIG_PATH), '--log-level', 'off'],
            capture_output=True,
            check=False,
            text=True,
            encoding='utf-8',
        )
        assert res_log_off.returncode == 0, f'--log-level off failed: {res_log_off.stderr}'
        combined_off = res_log_off.stdout + res_log_off.stderr
        assert 'INFO' not in combined_off, f'Logs should be silenced with --log-level off: {combined_off}'
        print('  [PASS] --log-level off suppressed all log output')

        res_log_info = subprocess.run(
            [str(binary), '--dry-run', '-c', str(CONFIG_PATH), '-l', 'info'],
            capture_output=True,
            check=False,
            text=True,
            encoding='utf-8',
        )
        assert res_log_info.returncode == 0, f'-l info failed: {res_log_info.stderr}'
        combined_info = res_log_info.stdout + res_log_info.stderr
        assert 'Starting RDNS client' in combined_info, f'Logs should appear with -l info: {combined_info}'
        print('  [PASS] -l info emitted standard operational logs')

        print('\nALL FUNCTIONAL TESTS PASSED SUCCESSFULLY!\n')

    finally:
        server_proc.terminate()
        try:
            server_proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            server_proc.kill()
        if STATE_FILE.exists():
            STATE_FILE.unlink()


if __name__ == '__main__':
    run_tests()
