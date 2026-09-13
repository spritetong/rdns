#!/usr/bin/env python3
# Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
# rdns is licensed under the GNU GPL v3.0 or later.

"""
Cloudflare Zone ID and Record ID Lookup Tool.

Queries the Cloudflare API v4 to retrieve Zone ID and DNS Record IDs
for RDNS configuration, using only Python's standard library.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from typing import Any

sys.dont_write_bytecode = True

API_BASE_URL = 'https://api.cloudflare.com/client/v4'


def http_get(
    endpoint: str,
    token: str,
    params: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Perform an authenticated GET request to the Cloudflare API v4."""
    url = f'{API_BASE_URL}{endpoint}'
    if params:
        query_str = urllib.parse.urlencode(params)
        url = f'{url}?{query_str}'

    headers = {
        'Authorization': f'Bearer {token}',
        'Content-Type': 'application/json',
        'User-Agent': 'rdns-cf-lookup/1.0',
    }

    req = urllib.request.Request(url, headers=headers, method='GET')
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            data = resp.read().decode('utf-8')
            return json.loads(data)
    except urllib.error.HTTPError as e:
        body = e.read().decode('utf-8', errors='replace')
        err_msg = f'HTTP {e.code} {e.reason}'
        try:
            err_json = json.loads(body)
            if err_json.get('errors'):
                messages = [
                    f'[{err.get("code")}] {err.get("message")}'
                    for err in err_json['errors']
                ]
                err_msg = '; '.join(messages)
        except (json.JSONDecodeError, KeyError, TypeError, ValueError):
            if body.strip():
                err_msg = f'{err_msg}: {body.strip()}'
        raise RuntimeError(err_msg) from e
    except urllib.error.URLError as e:
        raise RuntimeError(f'Network connection failed: {e.reason}') from e


def fetch_all_zones(token: str) -> list[dict[str, Any]]:
    """Retrieve all accessible zones for the token (handles pagination)."""
    zones: list[dict[str, Any]] = []
    page = 1
    while True:
        resp = http_get('/zones', token, {'page': page, 'per_page': 50})
        result = resp.get('result', [])
        zones.extend(result)
        result_info = resp.get('result_info', {})
        total_pages = result_info.get('total_pages', 1)
        if page >= total_pages or not result:
            break
        page += 1
    return zones


def find_matching_zone(
    zones: list[dict[str, Any]],
    domain: str,
) -> dict[str, Any] | None:
    """
    Find the most specific matching zone for a domain or subdomain.
    For example, if domain is 'ddns.example.com', it matches zone 'example.com'.
    """
    target = domain.strip().lower().rstrip('.')
    matches: list[dict[str, Any]] = []
    for z in zones:
        zone_name = z.get('name', '').strip().lower().rstrip('.')
        if target == zone_name or target.endswith(f'.{zone_name}'):
            matches.append(z)

    if not matches:
        return None

    # Pick the longest matching zone name (most specific)
    matches.sort(key=lambda x: len(x.get('name', '')), reverse=True)
    return matches[0]


def fetch_dns_records(zone_id: str, token: str) -> list[dict[str, Any]]:
    """Retrieve all DNS records for a given zone (handles pagination)."""
    records: list[dict[str, Any]] = []
    page = 1
    while True:
        resp = http_get(
            f'/zones/{zone_id}/dns_records',
            token,
            {'page': page, 'per_page': 100},
        )
        result = resp.get('result', [])
        records.extend(result)
        result_info = resp.get('result_info', {})
        total_pages = result_info.get('total_pages', 1)
        if page >= total_pages or not result:
            break
        page += 1
    return records


def print_table(headers: list[str], rows: list[list[str]]) -> None:
    """Print a clean formatted table to stdout."""
    if not rows:
        print('  (No matching records found)')
        return

    col_widths = [len(h) for h in headers]
    for row in rows:
        for idx, col in enumerate(row):
            if idx < len(col_widths):
                col_widths[idx] = max(col_widths[idx], len(col))

    header_line = '  '.join(f'{h:<{w}}' for h, w in zip(headers, col_widths))
    separator = '  '.join('-' * w for w in col_widths)
    print(header_line)
    print(separator)
    for row in rows:
        line = '  '.join(f'{col:<{w}}' for col, w in zip(row, col_widths))
        print(line)


def generate_rdns_yaml(
    zone_id: str,
    target_domain: str,
    records: list[dict[str, Any]],
) -> str:
    """Generate sample rdns task YAML configurations based on discovered records."""
    target_clean = target_domain.strip().lower().rstrip('.')

    # Group A and AAAA records by subdomain name
    grouped: dict[str, dict[str, str]] = {}
    for r in records:
        rtype = r.get('type', '').upper()
        if rtype in ('A', 'AAAA'):
            rname = r.get('name', '').strip().lower().rstrip('.')
            if rname not in grouped:
                grouped[rname] = {}
            grouped[rname][rtype] = r.get('id', '')

    if not grouped:
        return ''

    # If user specified a subdomain that exists in grouped, prioritize it
    selected_domains = (
        [target_clean] if target_clean in grouped else list(grouped.keys())
    )

    yaml_snippets: list[str] = []
    for d in selected_domains:
        rec_map = grouped[d]
        v4_id = rec_map.get('A')
        v6_id = rec_map.get('AAAA')
        sub_name = d.split('.')[0]

        if v4_id and v6_id:
            yaml_snippets.append(
                f"""  # Dual-stack (IPv4 + IPv6) atomic batch update
  - name: "cf-{sub_name}-dualstack"
    interface: "Local"
    domain: "{d}"
    provider: "cloudflare"
    args:
      token: "${{CF_API_TOKEN}}"
      zone_id: "{zone_id}"
      record_id_v4: "{v4_id}"
      record_id_v6: "{v6_id}"
"""
            )
        elif v4_id:
            yaml_snippets.append(
                f"""  # IPv4 A record update (adaptive)
  - name: "cf-{sub_name}-v4"
    interface: "Local"
    domain: "{d}"
    provider: "cloudflare"
    args:
      token: "${{CF_API_TOKEN}}"
      zone_id: "{zone_id}"
      record_id_v4: "{v4_id}"
"""
            )
        elif v6_id:
            yaml_snippets.append(
                f"""  # IPv6 AAAA record update (adaptive)
  - name: "cf-{sub_name}-v6"
    interface: "Local"
    domain: "{d}"
    provider: "cloudflare"
    args:
      token: "${{CF_API_TOKEN}}"
      zone_id: "{zone_id}"
      record_id_v6: "{v6_id}"
"""
            )

    return '\n'.join(yaml_snippets)


def main() -> None:
    if hasattr(sys.stdout, 'reconfigure'):
        sys.stdout.reconfigure(encoding='utf-8', errors='replace')  # type: ignore
    if hasattr(sys.stderr, 'reconfigure'):
        sys.stderr.reconfigure(encoding='utf-8', errors='replace')  # type: ignore

    parser = argparse.ArgumentParser(
        description='Query Cloudflare Zone ID and Record IDs for RDNS configuration using domain and token.',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""Examples:
  python scripts/cf_lookup.py
  python scripts/cf_lookup.py -t <TOKEN> -d example.com
  python scripts/cf_lookup.py -d sub.example.com
""",
    )
    parser.add_argument(
        '-t',
        '--token',
        dest='token',
        help='Cloudflare API Token (interactive prompt if omitted, or reads CF_API_TOKEN env var)',
    )
    parser.add_argument(
        '-d',
        '--domain',
        dest='domain',
        help='Target domain or subdomain (e.g. example.com or sub.example.com, interactive prompt if omitted)',
    )
    parser.add_argument(
        '-a',
        '--all',
        action='store_true',
        help='Display all DNS record types (CNAME, TXT, MX, etc.) instead of A/AAAA only',
    )

    args = parser.parse_args()

    # 1. Obtain Token
    token = (
        args.token
        or os.environ.get('CF_API_TOKEN')
        or os.environ.get('CLOUDFLARE_API_TOKEN')
    )
    if not token:
        try:
            token = input('Enter Cloudflare API Token: ').strip()
        except (KeyboardInterrupt, EOFError):
            print('\nInput canceled.')
            sys.exit(0)
        if not token:
            print('Error: Token cannot be empty!', file=sys.stderr)
            sys.exit(1)

    # 2. Obtain Domain
    domain = args.domain
    if not domain:
        try:
            domain = input(
                'Enter target domain or subdomain (e.g. example.com or sub.example.com): '
            ).strip()
        except (KeyboardInterrupt, EOFError):
            print('\nInput canceled.')
            sys.exit(0)
        if not domain:
            print('Error: Domain cannot be empty!', file=sys.stderr)
            sys.exit(1)

    domain = domain.strip().lower().rstrip('.')

    print('\n[+] Querying accessible zones from Cloudflare API...')
    try:
        zones = fetch_all_zones(token)
    except RuntimeError as e:
        print(f'[-] API authentication or request failed: {e}', file=sys.stderr)
        print(
            "    Please verify your token and ensure it has 'Zone - Zone - Read' permission.",
            file=sys.stderr,
        )
        sys.exit(1)

    if not zones:
        print(
            '[-] Error: Current token does not have access to any zones. Please check API token permissions.',
            file=sys.stderr,
        )
        sys.exit(1)

    # 3. Match Zone
    matched_zone = find_matching_zone(zones, domain)
    if not matched_zone:
        print(f"[-] No matching zone found for domain '{domain}'!", file=sys.stderr)
        print('[i] Accessible zones under this token:', file=sys.stderr)
        for z in zones:
            print(f'    - {z.get("name")} (Zone ID: {z.get("id")})', file=sys.stderr)
        sys.exit(1)

    zone_name = matched_zone['name']
    zone_id = matched_zone['id']

    print('=' * 80)
    print(f'  Matched Zone Domain : {zone_name}')
    print(f'  Zone ID             : {zone_id}')
    print('=' * 80)

    # 4. Fetch DNS records for the zone
    print(f'\n[+] Fetching all DNS records for Zone [{zone_name}]...')
    try:
        records = fetch_dns_records(zone_id, token)
    except RuntimeError as e:
        print(f'[-] Failed to fetch DNS records: {e}', file=sys.stderr)
        print(
            "    Please ensure token has 'Zone - DNS - Read' permission.",
            file=sys.stderr,
        )
        sys.exit(1)

    # 5. Filter and format display
    display_records = (
        records
        if args.all
        else [r for r in records if r.get('type', '').upper() in ('A', 'AAAA')]
    )

    # Sort: alphabetical by domain name, then by type A -> AAAA
    display_records.sort(key=lambda r: (r.get('name', ''), r.get('type', '')))

    headers = ['TYPE', 'RECORD ID', 'DOMAIN / SUBDOMAIN', 'CONTENT', 'PROXIED', 'TTL']
    rows: list[list[str]] = []
    for r in display_records:
        proxied_str = 'yes' if r.get('proxied') else 'no'
        ttl_str = 'auto' if r.get('ttl') == 1 else str(r.get('ttl', 'auto'))
        rows.append(
            [
                r.get('type', ''),
                r.get('id', ''),
                r.get('name', ''),
                r.get('content', ''),
                proxied_str,
                ttl_str,
            ]
        )

    print(
        f'\n[+] Found {len(display_records)} record(s)'
        + (' (all types):' if args.all else ' (A / AAAA records):')
    )
    print_table(headers, rows)

    # 6. Generate RDNS YAML configuration snippet
    yaml_config = generate_rdns_yaml(zone_id, domain, records)
    if yaml_config:
        print('\n' + '=' * 80)
        print('📋 Ready-to-use config.yaml task configuration for rdns:')
        print('=' * 80)
        print('tasks:')
        print(yaml_config.rstrip())
        print('=' * 80)
    else:
        print('\n[i] Notice: No A or AAAA records found in this zone.')
        print(
            '    To configure DDNS, please add an initial A or AAAA record in the Cloudflare dashboard first.'
        )


if __name__ == '__main__':
    main()
