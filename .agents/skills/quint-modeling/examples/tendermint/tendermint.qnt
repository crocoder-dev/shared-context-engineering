module tendermint {
  import basicSpells.* from "./spells/basicSpells"
  import choreo(processes = NODES) as choreo from "./choreo"

  type Round = int
  type Stage = ProposeStage | PreVoteStage | PreCommitStage | DecidedStage
  type Value = str

  type Bookkeeping = {
    evidence_propose: Set[ProposeMsg],
    evidence_prevote: Set[PreVoteMsg],
    evidence_precommit: Set[PreVoteMsg],
  }

  type TimeoutKind = ProposeTimeout | PreVoteTimeout | PreCommitTimeout
  type TimeoutEvent = { kind: TimeoutKind, round: Round }

  type ValueId = { hashed: Value }
  type Node = str

  type StateFields = {
    round: Round,
    stage: Stage,
    decision: Option[Value],
    locked_value: Option[Value],
    locked_round: Round,
    valid_value: Option[Value],
    valid_round: Round,
    after_prevote_for_first_time: bool,
    precommit_quorum: bool,
  }

  type ProposeMsg = {
    src: Node,
    round: Round,
    proposal: Value,
    valid_round: Round,
  }

  type PreVoteMsg = {
    src: Node,
    round: Round,
    id: Option[ValueId],
  }

  type Message =
    | Propose(ProposeMsg)
    | PreVote(PreVoteMsg)
    | PreCommit(PreVoteMsg)

  type CustomEffects = CollectEvidence(Message)
  type Event = TimeoutEvent
  type Extensions = Bookkeeping

  type LocalState = choreo::LocalState[Node, StateFields]
  type LocalContext = choreo::LocalContext[Node, StateFields, Message, Event, Extensions]
  type Transition = choreo::Transition[Node, StateFields, Message, Event, CustomEffects]
  type GlobalContext = choreo::GlobalContext[
    Node,
    StateFields,
    Message,
    Event,
    Extensions
  ]

  pure def get_proposals(messages: Set[Message]): Set[ProposeMsg] =
    messages.filterMap(m => {
      match m {
        | Propose(p) => Some(p)
        | _ => None
      }
    })

  pure def get_pre_votes(messages: Set[Message]): Set[PreVoteMsg] =
    messages.filterMap(m => {
      match m {
        | PreVote(p) => Some(p)
        | _ => None
      }
    })

  pure def get_pre_commits(messages: Set[Message]): Set[PreVoteMsg] =
    messages.filterMap(m => {
      match m {
        | PreCommit(p) => Some(p)
        | _ => None
      }
    })

  pure def source(message: Message): Node =
    match message {
      | Propose(p) => p.src
      | PreVote(p) => p.src
      | PreCommit(p) => p.src
    }

  pure def valid(v: Value): bool = {
    VALID_VALUES.contains(v)
  }

  pure def id(v: Value): ValueId = { hashed: v }

  pure def start_round(ctx: LocalContext, round: Round): Transition = {
    val s = ctx.state

    pure val effect =
      if (s.process_id == PROPOSER.get(round)) {
        pure val proposal =
          if (s.valid_value != None)
            s.valid_value.unwrap()
          else
            VALUES.get(round)

        choreo::Broadcast(
          Propose({ src: s.process_id, round: round, proposal: proposal, valid_round: s.valid_round })
        )
      } else {
        choreo::TriggerEvent({ kind: ProposeTimeout, round: s.round })
      }
    {
      effects: Set(effect),
      post_state: {
        ...s,
        round: round,
        stage: ProposeStage,
        precommit_quorum: false,
      }
    }
  }

  pure def listen_proposal_in_propose(ctx: LocalContext): Set[ProposeMsg] = {
    val s = ctx.state
    val messages = ctx.messages

    val state_guard = and {
      s.stage == ProposeStage
    }

    def message_guard = (p) => and {
      p.valid_round == -1,
      p.src == PROPOSER.get(s.round),
    }

    val proposals = messages.get_proposals()
    proposals.filter(p => and {
      message_guard(p),
      state_guard
    })
  }

  pure def broadcast_prevote_for_proposal(ctx: LocalContext, p: ProposeMsg): Transition = {
    val s = ctx.state

    pure val effects = if (valid(p.proposal) and (s.locked_round == -1 or s.locked_value == Some(p.proposal))) {
      Set(choreo::Broadcast(PreVote({ src: s.process_id, round: s.round, id: Some(id(p.proposal)) })))
    } else {
      Set(choreo::Broadcast(PreVote({ src: s.process_id, round: s.round, id: None })))
    }

    {
      post_state: { ...s, stage: PreVoteStage },
      effects: effects.setAdd(choreo::CustomEffect(CollectEvidence(Propose(p))))
    }
  }

  pure def listen_proposal_in_propose_prevote(ctx: LocalContext): Set[ProposeMsg] = {
    val s = ctx.state
    val messages = ctx.messages

    val state_guard = and {
      s.stage == ProposeStage,
    }

    def message_guard = (p) => and {
      p.src == PROPOSER.get(s.round),
      p.valid_round >= 0,
      p.valid_round < s.round
    }

    val proposals = messages.get_proposals().filter(p => and {
      message_guard(p),
      state_guard,
    })

    val pre_votes = messages.get_pre_votes()
    proposals.filter(p => and {
      pre_votes.filter(q => q.id == Some(p.proposal.id()) and q.round == p.valid_round).size() >= 2 * F + 1
    })
  }

  pure def broadcast_prevote_with_validation(ctx: LocalContext, p: ProposeMsg): Transition = {
    val s = ctx.state
    val messages = ctx.messages

    val contributing_prevotes = messages.get_pre_votes().filter(pv => 
      pv.round == p.valid_round and pv.id == Some(id(p.proposal))
    )
    val prevote_evidence = contributing_prevotes.map(pv => choreo::CustomEffect(CollectEvidence(PreVote(pv))))
    val evidence = Set(choreo::CustomEffect(CollectEvidence(Propose(p)))).union(prevote_evidence)

    val effects = if (valid(p.proposal) and (s.locked_round <= p.valid_round or s.locked_value == Some(p.proposal))) {
      Set(choreo::Broadcast(PreVote({ src: s.process_id, round: s.round, id: Some(id(p.proposal))})))
    } else {
      Set(choreo::Broadcast(PreVote({ src: s.process_id, round: s.round, id: None })))
    }

    {
      post_state: { ...s, stage: PreVoteStage },
      effects: effects.union(evidence)
    }
  }


  pure def listen_quorum_prevotes_any(ctx: LocalContext): Set[()] = {
    val s = ctx.state
    val messages = ctx.messages

    def message_guard = (pv) => (pv.round == s.round)
    val quorum = messages.get_pre_votes().filter(pv => and {
        message_guard(pv),
    }).size() >= 2 * F + 1

    if (quorum) {
      Set(())
    } else {
      Set()
    }
  }

  pure def trigger_prevote_timeout(ctx: LocalContext, _params: ()): Transition = {
    val s = ctx.state

    {
      post_state: s,
      effects: Set(choreo::TriggerEvent({ kind: PreVoteTimeout, round: s.round }))
    }
  }

  pure def listen_quorum_nil_prevotes(ctx: LocalContext): Set[()] = {
    val s = ctx.state
    val messages = ctx.messages

    val state_guard = and {
      s.stage == PreVoteStage,
    }

    def message_guard = (pv) => (pv.round == s.round and pv.id == None)
    val prevotes = messages.get_pre_votes().filter(pv => message_guard(pv))
    val quorum = prevotes.map(m => m.src).size() >= 2 * F + 1

    if (quorum and state_guard) {
      Set(())
    } else {
      Set()
    }
  }

  pure def broadcast_nil_precommit(ctx: LocalContext, _params: ()): Transition = {
    val s = ctx.state
    val messages = ctx.messages

    val nil_prevotes = messages.get_pre_votes().filter(pv => 
      pv.round == s.round and pv.id == None
    )
    val prevote_evidence = nil_prevotes.map(pv => choreo::CustomEffect(CollectEvidence(PreVote(pv))))

    {
      post_state: { ...s, stage: PreCommitStage },
      effects: Set(
        choreo::Broadcast(PreCommit({ src: s.process_id, round: s.round, id: None }))
      ).union(prevote_evidence)
    }
  }

  pure def listen_quorum_precommits_any(ctx: LocalContext): Set[()] = {
    val s = ctx.state
    val messages = ctx.messages

    def message_guard = (pc) => (pc.round == s.round)
    val quorum = messages.get_pre_commits().filter(pc => and {
      message_guard(pc),
    }).map(m => m.src).size() >= 2 * F + 1

    if (quorum) {
      Set(())
    } else {
      Set()
    }
  }

  pure def trigger_precommit_timeout(ctx: LocalContext, _params: ()): Transition = {
    val s = ctx.state

    {
      post_state: s,
      effects: Set(choreo::TriggerEvent({ kind: PreCommitTimeout, round: s.round }))
    }
  }

  pure def listen_proposal_in_prevote_commit(ctx: LocalContext): Set[ProposeMsg] = {
    val s = ctx.state
    val messages = ctx.messages

    val state_guard = and {
      s.stage.in(Set(PreVoteStage, PreCommitStage))
    }

    def message_guard = (p) => and {
      p.src == PROPOSER.get(s.round),
      p.round == s.round
    }

    val proposals = messages.get_proposals()
    val pre_votes = messages.get_pre_votes()

    proposals.filter(p => and {
      message_guard(p),
      state_guard,
      pre_votes.filter(pv => and {
        pv.round == s.round,
        pv.id == Some(id(p.proposal))
      }).map(pv => pv.src).size() >= 2 * F + 1
    })
  }

  pure def lock_value_and_precommit(ctx: LocalContext, p: ProposeMsg): Transition = {
    val s = ctx.state
    val messages = ctx.messages

    val enabling_prevotes = messages.get_pre_votes().filter(pv => 
      pv.round == s.round and pv.id == Some(id(p.proposal))
    )
    val prevote_evidence = enabling_prevotes.map(pv => choreo::CustomEffect(CollectEvidence(PreVote(pv))))

    if (s.stage == PreVoteStage) {
      {
        post_state: {
          ...s,
          stage: PreCommitStage,
          valid_value: Some(p.proposal),
          valid_round: s.round,
          locked_value: Some(p.proposal),
          locked_round: s.round
        },
        effects: Set(
          choreo::Broadcast(PreCommit({ src: s.process_id, round: s.round, id: Some(id(p.proposal)) })),
          choreo::CustomEffect(CollectEvidence(Propose(p)))
        ).union(prevote_evidence)
      }
    } else {
      {
        post_state: {
          ...s,
          valid_value: Some(p.proposal),
          valid_round: s.round
        },
        effects: Set(choreo::CustomEffect(CollectEvidence(Propose(p)))).union(prevote_evidence)
      }
    }
  }

  pure def listen_proposal_in_precommit_no_decision(ctx: LocalContext): Set[ProposeMsg] = {
    val s = ctx.state
    val messages = ctx.messages

    val state_guard = and {
      s.decision == None
    }

    def message_guard = (p) => and {
      p.src == PROPOSER.get(p.round),
      p.round <= s.round
    }

    val proposals = messages.get_proposals()
    val pre_commits = messages.get_pre_commits()

    proposals.filter(p => and {
      message_guard(p),
      state_guard,
      pre_commits.filter(pc => and {
        pc.round == p.round,
        pc.id == Some(id(p.proposal))
      }).map(pc => pc.src).size() >= 2 * F + 1
    })
  }

  pure def decide_on_proposal(ctx: LocalContext, p: ProposeMsg): Transition = {
    val s = ctx.state
    val messages = ctx.messages

    val enabling_precommits = messages.get_pre_commits().filter(pc =>
      pc.round == p.round and pc.id == Some(id(p.proposal))
    )
    val precommit_evidence = enabling_precommits.map(pc => choreo::CustomEffect(CollectEvidence(PreCommit(pc))))

    {
      post_state: {
        ...s,
        decision: Some(p.proposal),
        stage: DecidedStage
      },
      effects: Set(choreo::CustomEffect(CollectEvidence(Propose(p)))).union(precommit_evidence)
    }
  }

  pure def on_propose_timeout(ctx: LocalContext): Set[Transition] = {
    val s = ctx.state

    val candidates = ctx.events.filter(t => t.round >= s.round and t.kind == ProposeTimeout)
    val guard = and {
      candidates.size() > 0,
      s.stage == ProposeStage,
    }

    if (guard)
      Set({
        post_state: { ...s, stage: PreVoteStage },
        effects: Set(
          choreo::Broadcast(PreVote({ src: s.process_id, round: s.round, id: None })),
          choreo::TriggerEvent({ kind: PreVoteTimeout, round: s.round })
        ),
      })
    else
      Set()
  }

  pure def on_prevote_timeout(ctx: LocalContext): Set[Transition] = {
    val s = ctx.state

    val candidates = ctx.events.filter(t => t.round >= s.round and t.kind == PreVoteTimeout)
    val guard = and {
      candidates.size() > 0,
      s.stage == PreVoteStage,
    }

    if (guard)
      Set({
        post_state: { ...s, stage: PreCommitStage },
        effects: Set(
          choreo::Broadcast(PreVote({ src: s.process_id, round: s.round, id: None })),
          choreo::TriggerEvent({ kind: PreCommitTimeout, round: s.round })
        ),
      })
    else
      Set()
  }

  pure def on_precommit_timeout(ctx: LocalContext): Set[Transition] = {
    val s = ctx.state

    val candidates = ctx.events.filter(t => t.round >= s.round and t.kind == PreCommitTimeout)
    val guard = and {
      candidates.size() > 0,
      s.stage == PreCommitStage,
    }

    if (guard)
      Set(start_round(ctx, s.round + 1))
    else
      Set()
  }

  pure def main_listener(ctx: LocalContext): Set[Transition] =
    Set(
      choreo::cue(ctx, listen_proposal_in_propose, broadcast_prevote_for_proposal),
      choreo::cue(ctx, listen_proposal_in_propose_prevote, broadcast_prevote_with_validation),
      choreo::cue(ctx, listen_proposal_in_precommit_no_decision, decide_on_proposal),
      choreo::cue(ctx, listen_proposal_in_prevote_commit, lock_value_and_precommit),
      choreo::cue(ctx, listen_quorum_prevotes_any, trigger_prevote_timeout),
      choreo::cue(ctx, listen_quorum_nil_prevotes, broadcast_nil_precommit),
      choreo::cue(ctx, listen_quorum_precommits_any, trigger_precommit_timeout),
      on_propose_timeout(ctx),
      on_prevote_timeout(ctx),
      on_precommit_timeout(ctx)
    ).flatten()

  pure val initial_message =
    Propose({ src: PROPOSER.get(0), round: 0, proposal: VALUES.get(0), valid_round: -1 })

  pure val initial_bookkeeping = {
    evidence_propose: Set(),
    evidence_prevote: Set(),
    evidence_precommit: Set(),
  }

  def initialize_process(n: Node): LocalState = {
    {
      process_id: n,
      round: 0,
      stage: ProposeStage,
      decision: None,
      locked_value: None,
      locked_round: -1,
      valid_value: None,
      valid_round: -1,
      after_prevote_for_first_time: false,
      precommit_quorum: false,
    }
  }

  val byzantine_messages_1 = {
    val rounds = Set(0)
    val byzantine_proposals = rounds.map(r => {
      tuples(FAULTY, Set("v0","v1","v2"), rounds.setAdd(-1)).map(((f, v, vr)) => {
        Propose({ src: f, round: r, proposal: v, valid_round: vr })
      })
    }).flatten()

    val byzantine_prevotes = rounds.map(r => {
      tuples(FAULTY, Set("v0","v1","v2")).map(((f, v)) => {
        PreVote({ src: f, round: r, id: Some(v.id()) })
      })
    }).flatten()

    val byzantine_precommits = rounds.map(r => {
      tuples(FAULTY, Set("v0","v1","v2")).map(((f, v)) => {
        PreCommit({ src: f, round: r, id: Some(v.id()) })
      })
    }).flatten()

    Set(byzantine_prevotes, byzantine_precommits, byzantine_proposals).flatten()
  }

  val byzantine_messages = {

    val dummy_prevotes = 1000.to(1025).map(r => {
        PreVote({ src: "p4", round: r, id: None })
    })
    dummy_prevotes
  }

  action init = choreo::init({
    system: NODES.mapBy(n => initialize_process(n)),
    messages: NODES.mapBy(n => Set(initial_message).union(byzantine_messages_1)),
    events: NODES.mapBy(n => Set()),
    extensions: initial_bookkeeping
  })

  def apply_custom_effect(env: GlobalContext, effect: CustomEffects): GlobalContext = {
    match effect {
      | CollectEvidence(msg) => {
        pure val updated_bookkeeping = match msg {
          | Propose(p) => { ...env.extensions, evidence_propose: env.extensions.evidence_propose.setAdd(p) }
          | PreVote(p) => { ...env.extensions, evidence_prevote: env.extensions.evidence_prevote.setAdd(p) }
          | PreCommit(p) => { ...env.extensions, evidence_precommit: env.extensions.evidence_precommit.setAdd(p) }
        }
        { ...env, extensions: updated_bookkeeping }
      }
    }
  }

  action step = choreo::step(
    main_listener,
    apply_custom_effect
  )


  val correct_nodes = s.system.values().filter(s => s.process_id.in(CORRECT))
  val agreement = tuples(correct_nodes, correct_nodes).forall(((p1, p2)) => {
     p1.decision == None or p2.decision == None or p1.decision == p2.decision
  })

  val one_decided =
    not(s.system.values().forall(v => v.decision == None))

  val all_decided =
    not(correct_nodes.exists(v => v.decision == None))

  val stages =
    and {
      s.system.values().map(v => v.stage).contains(ProposeStage),
      s.system.values().map(v => v.stage).contains(PreVoteStage),
      s.system.values().map(v => v.stage).contains(PreCommitStage),
      s.system.values().map(v => v.stage).contains(DecidedStage)
    }

  pure def equivocation_in(n: Node, evidence: Set[{ src: Node, round: Round | other }]): bool = {
    tuples(evidence, evidence).exists(((e1, e2)) => {
      e1 != e2 and e1.src == n and e2.src == n and e1.round == e2.round
    })
  }

  def equivocation_by(n: Node): bool = {
    or {
      n.equivocation_in(s.extensions.evidence_propose),
      n.equivocation_in(s.extensions.evidence_prevote),
      n.equivocation_in(s.extensions.evidence_precommit),
    }
  }

  def amnesia_by(n: Node): bool = {
    tuples(ROUNDS, ROUNDS).exists(((r1, r2)) => {
      r1 < r2 and tuples(VALID_VALUES, VALID_VALUES).exists(((v1, v2)) => {
        and {
          v1 != v2,
          s.extensions.evidence_propose.contains({
            src: n, round: r1, proposal: v1, valid_round: r1
          }),
          s.extensions.evidence_propose.contains({
            src: n, round: r2, proposal: v2, valid_round: r2
          }),
          ROUNDS.filter(r => r1 <= r and r < r2).forall(r => {
            s.extensions.evidence_prevote.filter(m => {
              m.round == r and m.id == Some(id(v2))
            }).size() >= 2 * F + 1
          }),
        }
      })
    })
  }

  val validity = correct_nodes.forall(s => s.decision.in(VALID_VALUES.map(v => Some(v)).setAdd(None)))

  val accountability = or {
    agreement,
    FAULTY.powerset().exists(f => and {
      f.size() >= F + 1,
      f.forall(n => equivocation_by(n) or amnesia_by(n)),
    })
  }

  const F: int
  const CORRECT: Set[Node]
  const FAULTY: Set[Node]
  const PROPOSER: Round -> Node
  const VALUES: Round -> Value
  const MAX_ROUND: Round

  pure val NODES = CORRECT.union(FAULTY)
  pure val ROUNDS = VALUES.keys()

  pure val VALID_VALUES = Set("v0", "v1", "v2")

  action step_with(v: Node, listener: LocalContext => Set[Transition]): bool =
    choreo::step_with(v, listener, apply_custom_effect)

  action step_deterministic(v: Node, listener: LocalContext => Transition): bool =
    choreo::step_deterministic(v, listener, apply_custom_effect)

  def with_cue(process, listen_fn, params) =
    choreo::with_cue(process, listen_fn, params)

  action perform(cue_ctx, upon_fn) =
    choreo::perform(cue_ctx, upon_fn, apply_custom_effect)

  action step_with_filter(
    v: Node,
    listener: LocalContext => Set[Transition],
    f: Transition => bool
  ): bool =
    choreo::step_with_filter(v, listener, apply_custom_effect, f)

  action step_with_messages(
    v: Node,
    listener: LocalContext => Set[Transition],
    f: Set[Message] => Set[Message]
  ): bool =
    choreo::step_with_messages(v, listener, apply_custom_effect, f)

  action lose_messages = choreo::lose_messages

  val s = choreo::s
}

