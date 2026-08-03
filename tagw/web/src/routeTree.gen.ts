/* eslint-disable */

// @ts-nocheck

// noinspection JSUnusedGlobalSymbols

// You should NOT make any changes in this file as it will be overwritten.
// Additionally, you should also exclude this file from your linter and/or formatter to prevent it from being checked or modified.

import { Route as rootRouteImport } from './routes/__root'
import { Route as IndexRouteImport } from './routes/index'
import { Route as LoginRouteImport } from './routes/login'
import { Route as LogsRouteImport } from './routes/logs'
import { Route as MembersRouteImport } from './routes/members'
import { Route as ProvidersRouteImport } from './routes/providers'
import { Route as UsageRouteImport } from './routes/usage'
import { Route as AdminExportRouteImport } from './routes/admin.export'
import { Route as AdminKeysRouteImport } from './routes/admin.keys'

const IndexRoute = IndexRouteImport.update({
  id: '/',
  path: '/',
  getParentRoute: () => rootRouteImport,
})
const LoginRoute = LoginRouteImport.update({
  id: '/login',
  path: '/login',
  getParentRoute: () => rootRouteImport,
})
const LogsRoute = LogsRouteImport.update({
  id: '/logs',
  path: '/logs',
  getParentRoute: () => rootRouteImport,
})
const MembersRoute = MembersRouteImport.update({
  id: '/members',
  path: '/members',
  getParentRoute: () => rootRouteImport,
})
const ProvidersRoute = ProvidersRouteImport.update({
  id: '/providers',
  path: '/providers',
  getParentRoute: () => rootRouteImport,
})
const UsageRoute = UsageRouteImport.update({
  id: '/usage',
  path: '/usage',
  getParentRoute: () => rootRouteImport,
})
const AdminExportRoute = AdminExportRouteImport.update({
  id: '/admin/export',
  path: '/admin/export',
  getParentRoute: () => rootRouteImport,
})
const AdminKeysRoute = AdminKeysRouteImport.update({
  id: '/admin/keys',
  path: '/admin/keys',
  getParentRoute: () => rootRouteImport,
})

export interface FileRoutesByFullPath {
  '/': typeof IndexRoute
  '/login': typeof LoginRoute
  '/logs': typeof LogsRoute
  '/members': typeof MembersRoute
  '/providers': typeof ProvidersRoute
  '/usage': typeof UsageRoute
  '/admin/export': typeof AdminExportRoute
  '/admin/keys': typeof AdminKeysRoute
}
export interface FileRoutesByTo {
  '/': typeof IndexRoute
  '/login': typeof LoginRoute
  '/logs': typeof LogsRoute
  '/members': typeof MembersRoute
  '/providers': typeof ProvidersRoute
  '/usage': typeof UsageRoute
  '/admin/export': typeof AdminExportRoute
  '/admin/keys': typeof AdminKeysRoute
}
export interface FileRoutesById {
  __root__: typeof rootRouteImport
  '/': typeof IndexRoute
  '/login': typeof LoginRoute
  '/logs': typeof LogsRoute
  '/members': typeof MembersRoute
  '/providers': typeof ProvidersRoute
  '/usage': typeof UsageRoute
  '/admin/export': typeof AdminExportRoute
  '/admin/keys': typeof AdminKeysRoute
}
export interface FileRouteTypes {
  fileRoutesByFullPath: FileRoutesByFullPath
  fullPaths:
    | '/'
    | '/login'
    | '/logs'
    | '/members'
    | '/providers'
    | '/usage'
    | '/admin/export'
    | '/admin/keys'
  fileRoutesByTo: FileRoutesByTo
  to:
    | '/'
    | '/login'
    | '/logs'
    | '/members'
    | '/providers'
    | '/usage'
    | '/admin/export'
    | '/admin/keys'
  id:
    | '__root__'
    | '/'
    | '/login'
    | '/logs'
    | '/members'
    | '/providers'
    | '/usage'
    | '/admin/export'
    | '/admin/keys'
  fileRoutesById: FileRoutesById
}
export interface RootRouteChildren {
  IndexRoute: typeof IndexRoute
  LoginRoute: typeof LoginRoute
  LogsRoute: typeof LogsRoute
  MembersRoute: typeof MembersRoute
  ProvidersRoute: typeof ProvidersRoute
  UsageRoute: typeof UsageRoute
  AdminExportRoute: typeof AdminExportRoute
  AdminKeysRoute: typeof AdminKeysRoute
}

declare module '@tanstack/react-router' {
  interface FileRoutesByPath {
    '/': {
      id: '/'
      path: '/'
      fullPath: '/'
      preLoaderRoute: typeof IndexRouteImport
      parentRoute: typeof rootRouteImport
    }
    '/login': {
      id: '/login'
      path: '/login'
      fullPath: '/login'
      preLoaderRoute: typeof LoginRouteImport
      parentRoute: typeof rootRouteImport
    }
    '/logs': {
      id: '/logs'
      path: '/logs'
      fullPath: '/logs'
      preLoaderRoute: typeof LogsRouteImport
      parentRoute: typeof rootRouteImport
    }
    '/members': {
      id: '/members'
      path: '/members'
      fullPath: '/members'
      preLoaderRoute: typeof MembersRouteImport
      parentRoute: typeof rootRouteImport
    }
    '/providers': {
      id: '/providers'
      path: '/providers'
      fullPath: '/providers'
      preLoaderRoute: typeof ProvidersRouteImport
      parentRoute: typeof rootRouteImport
    }
    '/usage': {
      id: '/usage'
      path: '/usage'
      fullPath: '/usage'
      preLoaderRoute: typeof UsageRouteImport
      parentRoute: typeof rootRouteImport
    }
    '/admin/export': {
      id: '/admin/export'
      path: '/admin/export'
      fullPath: '/admin/export'
      preLoaderRoute: typeof AdminExportRouteImport
      parentRoute: typeof rootRouteImport
    }
    '/admin/keys': {
      id: '/admin/keys'
      path: '/admin/keys'
      fullPath: '/admin/keys'
      preLoaderRoute: typeof AdminKeysRouteImport
      parentRoute: typeof rootRouteImport
    }
  }
}

const rootRouteChildren: RootRouteChildren = {
  IndexRoute: IndexRoute,
  LoginRoute: LoginRoute,
  LogsRoute: LogsRoute,
  MembersRoute: MembersRoute,
  ProvidersRoute: ProvidersRoute,
  UsageRoute: UsageRoute,
  AdminExportRoute: AdminExportRoute,
  AdminKeysRoute: AdminKeysRoute,
}
export const routeTree = rootRouteImport
  ._addFileChildren(rootRouteChildren)
  ._addFileTypes<FileRouteTypes>()
