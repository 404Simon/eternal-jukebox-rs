export interface Beat {
  index: number
  start: number
  duration: number
}

export interface Analysis {
  duration: number
  sample_rate: number
  channels: number
  tempo: number
  beats: Beat[]
}

export interface Branch {
  destination: number
  distance: number
}

export interface BranchGraph {
  branches: Branch[][]
  threshold: number
  last_branch_point: number
}

export interface PreparedTrack {
  analysis: Analysis
  graph: BranchGraph
}

export interface PlannedBeat {
  beat: number
  jumpedFrom: number | null
  probability: number
}

export interface TransitionProbability {
  source: number
  destination: number
  probability: number
  distance: number | null
  is_sequential: boolean
}
