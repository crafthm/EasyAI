export type AppItem = {
  id: string
  name: string
  color: string
  glyph: string
}

// 演示数据，后续替换为真实应用列表
export const demoApps: AppItem[] = [
  {
    id: 'chat',
    name: 'Chat',
    color: 'linear-gradient(135deg, #6366f1, #8b5cf6)',
    glyph: 'C',
  },
  {
    id: 'notes',
    name: 'Notes',
    color: 'linear-gradient(135deg, #f59e0b, #f97316)',
    glyph: 'N',
  },
  {
    id: 'photos',
    name: 'Photos',
    color: 'linear-gradient(135deg, #ec4899, #f43f5e)',
    glyph: 'P',
  },
  {
    id: 'music',
    name: 'Music',
    color: 'linear-gradient(135deg, #10b981, #0d9488)',
    glyph: 'M',
  },
]