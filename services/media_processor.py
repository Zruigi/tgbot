# -*- coding: utf-8 -*-
"""
富媒体消息处理服务：下载、校验、转码与压缩
支持：Sticker, GIF, Animation, Video Note, Voice, Audio
"""
import os
import logging
import asyncio
import subprocess
import tempfile
import uuid
from typing import Optional, Tuple, Dict, Any
from PIL import Image
from telegram import Bot, File
import config

logger = logging.getLogger(__name__)

# 2026 规格：媒体限制
MAX_FILE_SIZE = 50 * 1024 * 1024  # 50 MB
MAX_DURATION = 60  # 60 seconds
MAX_RESOLUTION = (1280, 1280)
MAX_FPS = 30

class MediaProcessor:
    def __init__(self, bot: Bot):
        self.bot = bot
        self.temp_dir = tempfile.gettempdir()

    async def download_file(self, file_id: Optional[str]) -> str:
        """下载 Telegram 文件并返回本地路径"""
        if not file_id:
            raise ValueError("No file_id provided for download")
        file: File = await self.bot.get_file(file_id)
        file_path = file.file_path or "unknown.bin"
        ext = file_path.split('.')[-1]
        local_path = os.path.join(self.temp_dir, f"{uuid.uuid4()}.{ext}")
        await file.download_to_drive(local_path)
        return local_path

    def get_media_info(self, file_path: str) -> Dict[str, Any]:
        """使用 ffprobe 获取媒体信息"""
        try:
            cmd = [
                'ffprobe', '-v', 'quiet', '-print_format', 'json',
                '-show_format', '-show_streams', file_path
            ]
            result = subprocess.run(cmd, capture_output=True, text=True)
            if result.returncode != 0:
                return {}
            import json
            return json.loads(result.stdout)
        except Exception as e:
            logger.error(f"FFprobe error: {e}")
            return {}

    def validate_media(self, info: Dict[str, Any]) -> Tuple[bool, str]:
        """校验媒体文件是否符合规格"""
        if not info:
            return True, ""  # 无法获取信息时跳过校验或默认通过

        # 1. 大小校验
        size = int(info.get('format', {}).get('size', 0))
        if size > MAX_FILE_SIZE:
            return False, f"文件过大 ({size / 1024 / 1024:.1f}MB)，超过 50MB 限制"

        # 2. 时长校验
        duration = float(info.get('format', {}).get('duration', 0))
        if duration > MAX_DURATION:
            return False, f"时长过长 ({duration:.1f}s)，超过 60s 限制"

        # 3. 分辨率与帧率校验 (针对视频流)
        for stream in info.get('streams', []):
            if stream.get('codec_type') == 'video':
                width = int(stream.get('width', 0))
                height = int(stream.get('height', 0))
                if width > MAX_RESOLUTION[0] or height > MAX_RESOLUTION[1]:
                    return False, f"分辨率过高 ({width}x{height})，超过 1280x1280 限制"
                
                fps_eval = stream.get('r_frame_rate', '0/0')
                if '/' in fps_eval:
                    num, den = map(int, fps_eval.split('/'))
                    fps = num / den if den != 0 else 0
                    if fps > MAX_FPS:
                        return False, f"帧率过高 ({fps:.1f} FPS)，超过 30 FPS 限制"

        return True, ""

    async def transcode_to_mp4(self, input_path: str) -> Optional[str]:
        """将媒体文件转码为 MP4 (H.264 + AAC)"""
        output_path = os.path.join(self.temp_dir, f"{uuid.uuid4()}.mp4")
        
        # 针对 TGS (Lottie) 的特殊处理预留钩子
        if input_path.endswith('.tgs'):
            # TODO: 实现 TGS 转 MP4 (需要 rlottie 或类似工具)
            # 目前暂不支持 TGS 直接转码，先返回 None 触发兜底
            return None

        try:
            # FFmpeg 转码命令
            cmd = [
                'ffmpeg', '-y', '-i', input_path,
                '-c:v', 'libx264', '-preset', 'fast', '-crf', '23',
                '-c:a', 'aac', '-b:a', '128k',
                '-movflags', '+faststart', # 优化 Web 加载
                '-pix_fmt', 'yuv420p', # 兼容性最好的像素格式
                '-vf', "scale='min(1280,iw)':-2", # 自动缩放并保持偶数宽高
                output_path
            ]
            
            process = await asyncio.create_subprocess_exec(
                *cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
            )
            stdout, stderr = await process.communicate()
            
            if process.returncode == 0:
                return output_path
            else:
                logger.error(f"FFmpeg transcoding failed: {stderr.decode()}")
                return None
        except Exception as e:
            logger.error(f"Transcoding exception: {e}")
            return None

    async def generate_thumbnail(self, video_path: str) -> Optional[str]:
        """生成视频缩略图"""
        thumb_path = os.path.join(self.temp_dir, f"{uuid.uuid4()}.jpg")
        try:
            cmd = [
                'ffmpeg', '-y', '-i', video_path,
                '-ss', '00:00:01', '-vframes', '1',
                '-q:v', '2', thumb_path
            ]
            process = await asyncio.create_subprocess_exec(
                *cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
            )
            await process.communicate()
            if os.path.exists(thumb_path):
                return thumb_path
        except Exception as e:
            logger.error(f"Thumbnail generation error: {e}")
        return None

    async def process_media(self, file_id: Optional[str], msg_type: str) -> Tuple[bool, Optional[str], Optional[str], str]:
        """
        完整处理流程：下载 -> 校验 -> 转码 -> 缩略图
        返回: (是否成功, 处理后的路径, 缩略图路径, 错误信息)
        """
        if not file_id:
            return False, None, None, "No file_id provided"
            
        local_path = None
        processed_path = None
        thumb_path = None
        
        try:
            # 1. 下载
            local_path = await self.download_file(file_id)
            
            # 2. 获取信息并校验
            info = self.get_media_info(local_path)
            if info:
                is_valid, error_msg = self.validate_media(info)
                if not is_valid:
                    # 告警日志
                    logger.warning(f"🚨 [Media-Alert] 媒体文件校验不通过: {error_msg} (FileID: {file_id})")
                    return False, None, None, error_msg
            
            # 3. 判断是否需要转码
            ext = local_path.split('.')[-1].lower()
            needs_transcode = ext in ['webm', 'tgs', 'gif', 'mov']
            
            if needs_transcode:
                logger.info(f"🔄 [Transcode] 正在转码 {ext} -> MP4...")
                processed_path = await self.transcode_to_mp4(local_path)
                if not processed_path:
                    logger.warning(f"⚠️ [Transcode-Fallback] 转码失败，将尝试原始文件转发 (Ext: {ext})")
                    processed_path = local_path
            else:
                processed_path = local_path
            
            # 4. 生成缩略图
            if msg_type in ['video', 'animation', 'video_note']:
                thumb_path = await self.generate_thumbnail(processed_path)
            
            return True, processed_path, thumb_path, ""
            
        except Exception as e:
            logger.error(f"❌ [Media-Error] 处理失败: {e} (FileID: {file_id})")
            return False, None, None, str(e)
        finally:
            if local_path and local_path != processed_path and os.path.exists(local_path):
                os.remove(local_path)

    def cleanup(self, *paths):
        """清理临时文件"""
        for path in paths:
            if path and os.path.exists(path):
                try:
                    os.remove(path)
                except Exception as e:
                    logger.warning(f"Failed to cleanup temp file {path}: {e}")
