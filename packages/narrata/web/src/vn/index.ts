// The public VN entry contains only the reader runtime; authoring, player UI and graph tools
// remain separate capabilities and are not loaded by the root or Gamebook reader entries.
export { openVn, checkedVnView, decodeVnView } from "./runtime.js";
export type { OpenVnOptions, VnReader, VnDependencyLimits, VnDependencyResult, VnCandidatePreparation, CandidateObjectStatus } from "./runtime.js";
export type {
  DependencyContentRoleV1,
  DependencyMediaRoleV1,
  DependencyTruncationV1,
  DependencyUnknownV1,
  DependencyRequestV1,
  DependencyLimitsV1,
  DependencyResultV1,
  DependencyContentV1,
  DependencyGraphV1,
  DependencyMediaV1,
  DependencyWorkV1,
  VnDependenciesV1,
} from "../generated/vn-dependencies.js";
export type * from "../generated/vn-view.js";
