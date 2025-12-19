export type OrgRole = "member" | "admin";

export type MeResponse = {
  id: number;
  email: string;
  name?: string | null;
  github_username?: string | null;
  created_at: string;
};

export type OrganizationEntry = {
  id: number;
  name: string;
  role: OrgRole;
  created_at: string;
};

export type OrganizationListResponse = {
  organizations: OrganizationEntry[];
};

export type CreateOrganizationResponse = {
  id: number;
  name: string;
};

export type MemberEntry = {
  account_id: number;
  email: string;
  name?: string | null;
  role: OrgRole;
  joined_at: string;
  bot: boolean;
};

export type MemberListResponse = {
  members: MemberEntry[];
};

export type OrgApiKeyEntry = {
  id: number;
  name: string;
  account_id: number;
  account_email: string;
  bot: boolean;
  created_at: string;
  accessed_at: string;
};

export type OrgApiKeyListResponse = {
  api_keys: OrgApiKeyEntry[];
};

export type CreateOrgApiKeyResponse = {
  id: number;
  name: string;
  token: string;
  created_at: string;
};

export type InvitationPreviewResponse = {
  organization_name: string;
  role: OrgRole;
  expires_at?: string | null;
  valid: boolean;
};

export type InvitationEntry = {
  id: number;
  role: OrgRole;
  created_at: string;
  expires_at?: string | null;
  max_uses?: number | null;
  use_count: number;
  revoked: boolean;
};

export type InvitationListResponse = {
  invitations: InvitationEntry[];
};

export type CreateInvitationResponse = {
  id: number;
  token: string;
  role: OrgRole;
  expires_at?: string | null;
  max_uses?: number | null;
};

export type AcceptInvitationResponse = {
  organization_id: number;
  organization_name: string;
  role: OrgRole;
};

export type BotEntry = {
  account_id: number;
  name?: string | null;
  responsible_email: string;
  created_at: string;
};

export type BotListResponse = {
  bots: BotEntry[];
};

export type CreateBotResponse = {
  account_id: number;
  name: string;
  api_key: string;
};

export type ExchangeResponse = {
  session_token: string;
};

/** Audit log entry. Fields use snake_case to match API response structure. */
export type AuditLogEntry = {
  id: number;
  account_id?: number | null;
  account_email?: string | null;
  account_name?: string | null;
  action: string;
  details?: Record<string, unknown> | null;
  created_at: string;
};

export type AuditLogListResponse = {
  entries: AuditLogEntry[];
  has_more: boolean;
};
