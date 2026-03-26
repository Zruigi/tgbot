---
name: "port-8080-monitor"
description: "Monitors local port 8080 for web application issues and provides AI diagnosis. Invoke when user reports errors on port 8080 or demo fails."
---

# Port 8080 Monitor (Demo Issue Tracker)

This skill is designed to monitor the web application running on port 8080 (admin_new.html) and provide AI-driven diagnostics for common errors like WebSocket disconnects, API fetch failures, and JavaScript scoping issues.

## Usage

Invoke this skill when:
- The user reports a `TypeError` in the browser console.
- The web management interface fails to load (`ERR_CONNECTION_REFUSED`).
- WebSocket messages are not being received or handled correctly.
- Any "demo" related issues occur on port 8080.

## Diagnostic Checklist

1. **Server Status**: Check if the backend (web_admin.py) is running and listening on 8080.
2. **WebSocket Connectivity**: Verify the `/ws` endpoint is active and the frontend is connecting using the correct protocol (ws/wss).
3. **JS Context (this)**: Ensure WebSocket callbacks in `admin_new.html` use arrow functions or `bind(this)` to maintain the Vue instance context.
4. **API Endpoints**: Validate that `/api/users`, `/api/monitors`, and `/api/potential_users` are returning valid JSON.
5. **Network Access**: Check for firewall or proxy issues if `ERR_CONNECTION_REFUSED` persists.

## Common Solutions

- **this.loadMonitors is not a function**: Usually caused by a lost `this` context in WebSocket `onmessage` or `onclose` callbacks. Use `const self = this;` or arrow functions.
- **ERR_CONNECTION_REFUSED**: The Python process might have crashed. Restart the bot manually or check port usage with `netstat -aon | findstr :8080`.
- **WebSocket error**: Ensure the `aiohttp` server is properly configured to handle WebSocket upgrades.
