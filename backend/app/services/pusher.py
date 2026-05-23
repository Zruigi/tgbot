# -*- coding: utf-8 -*-
"""
WebSocket 消息分发服务
"""
import asyncio
import logging
import json
from typing import Dict, Set, Any
from aiohttp import web

logger = logging.getLogger(__name__)


class PusherService:
    """WebSocket 消息分发服务"""
    
    def __init__(self):
        self._connections: Dict[int, Set[web.WebSocketResponse]] = {}
        self._lock = asyncio.Lock()
    
    async def add_connection(
        self,
        user_id: int,
        ws: web.WebSocketResponse
    ) -> None:
        """添加 WebSocket 连接"""
        async with self._lock:
            if user_id not in self._connections:
                self._connections[user_id] = set()
            self._connections[user_id].add(ws)
            logger.info(f"WebSocket 连接已添加：user_id={user_id}")
    
    async def remove_connection(
        self,
        user_id: int,
        ws: web.WebSocketResponse
    ) -> None:
        """移除 WebSocket 连接"""
        async with self._lock:
            if user_id in self._connections:
                self._connections[user_id].discard(ws)
                if not self._connections[user_id]:
                    del self._connections[user_id]
                logger.info(f"WebSocket 连接已移除：user_id={user_id}")
    
    async def push_to_user(
        self,
        user_id: int,
        message: Dict[str, Any]
    ) -> int:
        """
        推送消息给用户
        
        Args:
            user_id: 用户 ID
            message: 消息内容
        
        Returns:
            成功推送的连接数
        """
        async with self._lock:
            connections = self._connections.get(user_id, set()).copy()
        
        if not connections:
            logger.debug(f"用户无活跃连接：user_id={user_id}")
            return 0
        
        success_count = 0
        for ws in connections:
            try:
                await ws.send_str(json.dumps(message))
                success_count += 1
            except Exception as e:
                logger.error(f"推送失败：user_id={user_id}, error={e}")
                # 连接已断开，从列表中移除
                await self.remove_connection(user_id, ws)
        
        return success_count
    
    async def broadcast(
        self,
        message: Dict[str, Any]
    ) -> int:
        """
        广播消息给所有连接
        
        Args:
            message: 消息内容
        
        Returns:
            成功推送的连接数
        """
        async with self._lock:
            all_connections = []
            for connections in self._connections.values():
                all_connections.extend(connections)
        
        success_count = 0
        for ws in all_connections:
            try:
                await ws.send_str(json.dumps(message))
                success_count += 1
            except Exception as e:
                logger.error(f"广播失败：error={e}")
        
        return success_count
    
    def get_stats(self) -> Dict[str, Any]:
        """获取统计信息"""
        return {
            'total_users': len(self._connections),
            'total_connections': sum(len(conns) for conns in self._connections.values()),
            'connections_by_user': {
                uid: len(conns) for uid, conns in self._connections.items()
            }
        }


# 全局单例
_pusher_service = PusherService()


def get_pusher_service() -> PusherService:
    """获取 Pusher 服务实例"""
    return _pusher_service
