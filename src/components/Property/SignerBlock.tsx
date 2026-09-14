import { ShieldAlert, ShieldCheck, ShieldQuestion } from 'lucide-react'
import type { SignerInfo } from '@/types/model'
import { Badge } from '@/components/common/Badge'
import { Field } from '@/components/common/Field'

/**
 * 签名信息的展示。
 *
 * ⚠️ 这里的判断顺序不是随意的——**系统组件必须排在最前面**。
 *
 * 原因是一个真实存在的坑：Win11 上 `cmd.exe`、`notepad.exe` 已经变成
 * Store 应用的「执行别名」（reparse point）。`System32` 下剩下的只是一个
 * 转发的 stub，`WinVerifyTrust` 对它返回 `TRUST_E_NOSIGNATURE`。
 * 如果照直显示"未签名"，用户会看到"微软的系统程序没有签名"这种荒谬结论。
 *
 * 数据层保持诚实（`isSigned` 如实反映验签结果），**误导在这里被拦掉**。
 */
export function SignerBlock({ signer }: { signer: SignerInfo }) {
  const isSystem = signer.isOsComponent

  return (
    <div>
      <div className="mb-1 flex items-center gap-1.5">
        {isSystem ? (
          <Badge color="#58a6ff">
            <ShieldCheck size={11} />
            Windows 系统文件
          </Badge>
        ) : signer.certValid === false ? (
          <Badge color="#d29922">
            <ShieldAlert size={11} />
            签名已失效
          </Badge>
        ) : signer.isSigned ? (
          <Badge color="#3fb950">
            <ShieldCheck size={11} />
            签名有效
          </Badge>
        ) : (
          <Badge color="#d29922">
            <ShieldQuestion size={11} />
            未签名
          </Badge>
        )}

        {!isSystem && signer.isMicrosoft && <Badge color="#58a6ff">微软发布</Badge>}
      </div>

      {signer.certValid === false && (
        <p className="mb-1.5 text-mini leading-relaxed text-ink-dim">
          它能读到签名，但证书链没通过校验（通常是证书过期）。这不等同于「没签名」。
        </p>
      )}

      <Field label="发布者" labelWidth="w-12">
        {signer.publisher ?? <span className="text-ink-dim">无法核验</span>}
      </Field>
    </div>
  )
}
