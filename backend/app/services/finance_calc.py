# -*- coding: utf-8 -*-
"""
财务计算服务 - 财务报表生成、对账
"""
import logging
from typing import List, Dict, Any
from decimal import Decimal

logger = logging.getLogger(__name__)


class FinanceCalcService:
    """财务计算服务"""
    
    def calculate_pnl(
        self,
        buy_price: Decimal,
        sell_price: Decimal,
        quantity: Decimal,
        fee_rate: Decimal = Decimal('0.001')
    ) -> Dict[str, Decimal]:
        """
        计算盈亏
        
        Args:
            buy_price: 买入价格
            sell_price: 卖出价格
            quantity: 数量
            fee_rate: 手续费率
        
        Returns:
            盈亏详情
        """
        cost = buy_price * quantity
        revenue = sell_price * quantity
        fee = (cost + revenue) * fee_rate
        pnl = revenue - cost - fee
        pnl_rate = (pnl / cost * 100) if cost > 0 else Decimal('0')
        
        return {
            'cost': cost,
            'revenue': revenue,
            'fee': fee,
            'pnl': pnl,
            'pnl_rate': pnl_rate
        }
    
    def generate_summary(
        self,
        transactions: List[Dict[str, Any]]
    ) -> Dict[str, Any]:
        """
        生成财务摘要
        
        Args:
            transactions: 交易记录列表
        
        Returns:
            财务摘要统计
        """
        total_cost = Decimal('0')
        total_revenue = Decimal('0')
        total_fee = Decimal('0')
        profitable_count = 0
        losing_count = 0
        
        for tx in transactions:
            result = self.calculate_pnl(
                buy_price=Decimal(str(tx['buy_price'])),
                sell_price=Decimal(str(tx['sell_price'])),
                quantity=Decimal(str(tx['quantity']))
            )
            
            total_cost += result['cost']
            total_revenue += result['revenue']
            total_fee += result['fee']
            
            if result['pnl'] > 0:
                profitable_count += 1
            else:
                losing_count += 1
        
        total_pnl = total_revenue - total_cost - total_fee
        win_rate = (profitable_count / len(transactions) * 100) if transactions else Decimal('0')
        
        return {
            'total_transactions': len(transactions),
            'total_cost': float(total_cost),
            'total_revenue': float(total_revenue),
            'total_fee': float(total_fee),
            'total_pnl': float(total_pnl),
            'win_rate': float(win_rate),
            'profitable_count': profitable_count,
            'losing_count': losing_count
        }
    
    def reconcile(
        self,
        internal_records: List[Dict],
        external_records: List[Dict]
    ) -> Dict[str, Any]:
        """
        对账
        
        Args:
            internal_records: 内部记录
            external_records: 外部记录
        
        Returns:
            对账结果
        """
        # 按交易 ID 分组
        internal_map = {r['tx_id']: r for r in internal_records}
        external_map = {r['tx_id']: r for r in external_records}
        
        matched = []
        unmatched_internal = []
        unmatched_external = []
        
        # 查找匹配的记录
        for tx_id, internal in internal_map.items():
            if tx_id in external_map:
                external = external_map[tx_id]
                if self._is_match(internal, external):
                    matched.append({
                        'tx_id': tx_id,
                        'internal': internal,
                        'external': external
                    })
                else:
                    unmatched_internal.append(internal)
                    unmatched_external.append(external)
            else:
                unmatched_internal.append(internal)
        
        # 查找外部独有的记录
        for tx_id, external in external_map.items():
            if tx_id not in internal_map:
                unmatched_external.append(external)
        
        return {
            'matched_count': len(matched),
            'unmatched_internal_count': len(unmatched_internal),
            'unmatched_external_count': len(unmatched_external),
            'matched': matched,
            'unmatched_internal': unmatched_internal,
            'unmatched_external': unmatched_external
        }
    
    def _is_match(self, record1: Dict, record2: Dict) -> bool:
        """检查两条记录是否匹配"""
        tolerance = Decimal('0.01')  # 允许 1% 的误差
        
        amount1 = Decimal(str(record1.get('amount', 0)))
        amount2 = Decimal(str(record2.get('amount', 0)))
        
        diff = abs(amount1 - amount2)
        return diff <= tolerance
