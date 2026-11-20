import {expect,it} from 'vitest';
import {errorMessage} from './errorRecovery';
it('network failures have a recovery action while server validation and conflict explanations are retained',()=>{
 for(const value of ['Failed to fetch','Load failed','NetworkError when attempting to fetch resource.'])expect(errorMessage(new TypeError(value))).toBe('无法连接核心，请检查网络或等待重连后重试');
 expect(errorMessage(new Error('缓存预算不能低于当前占用'))).toBe('缓存预算不能低于当前占用');
 expect(errorMessage(new Error('身份已改变，请重新读取'))).toBe('身份已改变，请重新读取');
});
it('explicit cancellation and unknown failures remain readable',()=>{
 expect(errorMessage(new DOMException('aborted','AbortError'))).toBe('操作已取消，请重试');expect(errorMessage(null)).toBe('操作失败，请重试');
});
