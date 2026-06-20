/**
 * n8n Custom Nodes for TGBot
 * 
 * This module provides n8n nodes for:
 * - AI Agent integration
 * - Advertisement detection and management
 * - Broadcast messaging
 * - Channel posting
 */

module.exports = function() {
  const nodes = [
    require('./nodes/TgbotAiAgent'),
    require('./nodes/TgbotAdDetect'),
    require('./nodes/TgbotAdCrud'),
    require('./nodes/TgbotBroadcast'),
    require('./nodes/TgbotChannelPost'),
  ];
  
  const credentials = [
    require('./credentials/TgbotApi'),
  ];
  
  return { nodes, credentials };
};
