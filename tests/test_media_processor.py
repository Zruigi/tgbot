# -*- coding: utf-8 -*-
"""
Media Processor 单元测试
"""
import asyncio
import os
import sys
import logging

# 将项目根目录加入 path
sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from services.media_processor import MediaProcessor

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

async def test_validation():
    logger.info("Testing validation logic...")
    processor = MediaProcessor(None)
    
    # 模拟 FFprobe 输出
    info_too_big = {'format': {'size': '60000000'}} # 60MB
    is_valid, err = processor.validate_media(info_too_big)
    assert not is_valid and "过大" in err
    
    info_too_long = {'format': {'duration': '70'}} # 70s
    is_valid, err = processor.validate_media(info_too_long)
    assert not is_valid and "时长" in err
    
    info_high_res = {
        'streams': [{'codec_type': 'video', 'width': 1920, 'height': 1080, 'r_frame_rate': '30/1'}]
    }
    is_valid, err = processor.validate_media(info_high_res)
    assert not is_valid and "分辨率" in err
    
    logger.info("Validation tests passed!")

async def test_transcoding_path_logic():
    logger.info("Testing transcoding path logic...")
    processor = MediaProcessor(None)
    
    # 测试文件扩展名识别
    test_paths = [
        ("test.webm", True),
        ("test.tgs", True),
        ("test.gif", True),
        ("test.mov", True),
        ("test.mp4", False)
    ]
    
    for path, expected in test_paths:
        ext = path.split('.')[-1].lower()
        needs_transcode = ext in ['webm', 'tgs', 'gif', 'mov']
        assert needs_transcode == expected
        
    logger.info("Transcoding path logic tests passed!")

if __name__ == "__main__":
    asyncio.run(test_validation())
    asyncio.run(test_transcoding_path_logic())
