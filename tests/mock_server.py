# Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
# rdns is licensed under the GNU GPL v3.0 or later.

"""
Mock HTTP server for RDNS functional testing using FastAPI and Uvicorn.
"""

import sys

import uvicorn
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, PlainTextResponse

app = FastAPI(title='RDNS Mock Server')

# In-memory request history
records = []


@app.middleware('http')
async def record_requests(request: Request, call_next):
    body = await request.body()
    record = {
        'method': request.method,
        'path': request.url.path,
        'query': str(request.url.query),
        'headers': dict(request.headers),
        'body': body.decode('utf-8', errors='replace'),
    }
    # Do not record the query for records/reset
    if not request.url.path.startswith(
        '/api/records'
    ) and not request.url.path.startswith('/api/reset'):
        records.append(record)
    response = await call_next(request)
    return response


@app.get('/health')
def health():
    return {'status': 'ok'}


@app.get('/ip/v4')
def mock_ipv4():
    return PlainTextResponse('203.0.113.195')


@app.get('/ip/v6')
def mock_ipv6():
    return PlainTextResponse('2001:db8::1234:5678')


@app.get('/nic/update')
def mock_dynu_update(
    hostname: str = '',
    myip: str = '',
    myipv6: str = '',
    password: str = '',
    username: str = '',
    group: str = '',
):
    return PlainTextResponse('good 203.0.113.195')


@app.post('/api/v1/ddns/update')
def mock_webhook_update():
    return JSONResponse({'status': 'success', 'message': 'Record updated'})


@app.post('/api/v1/notify')
def mock_notification():
    return JSONResponse({'status': 'received'})


@app.get('/api/records')
def get_records():
    return JSONResponse(records)


@app.post('/api/reset')
def reset_records():
    records.clear()
    return JSONResponse({'status': 'cleared'})


if __name__ == '__main__':
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8899
    uvicorn.run(app, host='127.0.0.1', port=port, log_level='warning')
