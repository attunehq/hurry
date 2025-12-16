import { useOutletContext } from "react-router";
import type { OrgRole } from "../../api/types";

export type OrgOutletContext = {
  orgId: number;
  role: OrgRole | null;
};

export function useOrgContext() {
  return useOutletContext<OrgOutletContext>();
}
