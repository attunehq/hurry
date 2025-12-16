import { Navigate } from "react-router";

export default function OrgIndexRedirect() {
  return <Navigate to="members" replace />;
}
