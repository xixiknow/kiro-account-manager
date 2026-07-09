import { lazy, LazyExoticComponent, ComponentType } from 'react'
import {
  Gauge,
  KeyRound,
  LogIn,
  ListChecks,
  MessageSquare,
  Network,
  Settings,
  LucideIcon,
} from 'lucide-react'

export interface ServerRouteConfig {
  id: string
  icon: LucideIcon
  label: string
  desc: string
  component: LazyExoticComponent<ComponentType<any>>
}

export const serverRoutes: ServerRouteConfig[] = [
  {
    id: 'home',
    icon: Gauge,
    label: '仪表盘',
    desc: '账号、配额和运行态',
    component: lazy(() => import('./components/features/Home/index')),
  },
  {
    id: 'accounts',
    icon: KeyRound,
    label: '账号管理',
    desc: '批量、标签、代理、模型',
    component: lazy(() => import('./components/features/AccountManager/index')),
  },
  {
    id: 'desktopOAuth',
    icon: LogIn,
    label: '在线登录',
    desc: 'Google/GitHub/BuilderId/IAM',
    component: lazy(() => import('./components/features/Login/index')),
  },
  {
    id: 'rules',
    icon: ListChecks,
    label: '规则管理',
    desc: '模型映射、过滤和路由',
    component: lazy(() => import('./components/server/GatewayRules')),
  },
  {
    id: 'sessions',
    icon: MessageSquare,
    label: '会话管理',
    desc: 'Workspace 与历史会话',
    component: lazy(() => import('./components/features/SessionManager/index')),
  },
  {
    id: 'gateway',
    icon: Network,
    label: 'Kiro2Api',
    desc: '网关、日志和 Prompt Cache',
    component: lazy(() => import('./components/features/Gateway/index')),
  },
  {
    id: 'settings',
    icon: Settings,
    label: '设置',
    desc: '服务端、主题和应用设置',
    component: lazy(() => import('./components/server/ServerSettings')),
  },
]
