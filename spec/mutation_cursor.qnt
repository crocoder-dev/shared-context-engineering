module mutation_cursor {
  type WorktreeId = WT0 | WT1
  type ActorKind = ClaudeCode | Codex | OpenCode | Pi
  type ScopeId = Scope0 | Scope1 | Scope2 | Scope3
  type TreeId = Tree0 | Tree1 | Tree2 | Tree3
  type EventId =
    | Event0
    | Event1
    | Event2
    | Event3
    | Event4
    | Event5
    | Event6
    | Event7
    | Event8
    | Event9
  type AttemptId = Attempt0 | Attempt1 | Attempt2 | Attempt3 | Attempt4 | Attempt5
  type EventKey = {
    scopeId: ScopeId,
    eventId: EventId,
  }
  type FailureKind = Healthy | SnapshotFailure | DatabaseFailure

  type ScopeStatus = NeverSeen | Active | Closed
  type AttemptStatus = Available | Prepared | Committed | Rejected
  type Attribution =
    | IneligibleUnscoped
    | AiExclusive(ScopeId)
    | AiContended

  type Boundary =
    | Start({ scope: ScopeId, event: EventId })
    | Advance({ scope: ScopeId, event: EventId })
    | Close({ scope: ScopeId, event: EventId })
    | Flush(WorktreeId)

  type WorktreeState = {
    cursorTree: TreeId,
    revision: int,
    tainted: bool,
    failureKind: FailureKind,
  }

  type ScopeState = {
    status: ScopeStatus,
    actorKind: ActorKind,
    worktreeId: WorktreeId,
  }

  type AttemptState = {
    status: AttemptStatus,
    boundary: Boundary,
    expectedRevision: int,
    beforeTree: TreeId,
    afterTree: TreeId,
  }

  type CursorCheckpoint = {
    worktreeId: WorktreeId,
    revision: int,
    cursorTree: TreeId,
  }

  type MutationEvent = {
    worktreeId: WorktreeId,
    revision: int,
    beforeTree: TreeId,
    afterTree: TreeId,
    activeScopes: Set[ScopeId],
    tainted: bool,
    failureKind: FailureKind,
    attribution: Attribution,
    boundary: Boundary,
  }

  val WORKTREES: Set[WorktreeId] = Set(WT0, WT1)
  val SCOPES: Set[ScopeId] = Set(Scope0, Scope1, Scope2, Scope3)
  val TREES: Set[TreeId] = Set(Tree0, Tree1, Tree2, Tree3)
  val EVENTS: Set[EventId] = Set(
    Event0,
    Event1,
    Event2,
    Event3,
    Event4,
    Event5,
    Event6,
    Event7,
    Event8,
    Event9
  )
  val ATTEMPTS: Set[AttemptId] = Set(
    Attempt0,
    Attempt1,
    Attempt2,
    Attempt3,
    Attempt4,
    Attempt5
  )

  pure def scopeWorktree(scope: ScopeId): WorktreeId =
    match scope {
      | Scope0 => WT0
      | Scope1 => WT0
      | Scope2 => WT0
      | Scope3 => WT1
    }

  pure def scopeActor(scope: ScopeId): ActorKind =
    match scope {
      | Scope0 => ClaudeCode
      | Scope1 => ClaudeCode
      | Scope2 => Codex
      | Scope3 => OpenCode
    }

  pure def isLive(status: ScopeStatus): bool = status == Active

  pure def isTerminal(status: ScopeStatus): bool = status == Closed

  pure def boundaryWorktree(boundary: Boundary): WorktreeId =
    match boundary {
      | Start(data) => scopeWorktree(data.scope)
      | Advance(data) => scopeWorktree(data.scope)
      | Close(data) => scopeWorktree(data.scope)
      | Flush(worktree) => worktree
    }

  pure def boundaryScope(boundary: Boundary): ScopeId =
    match boundary {
      | Start(data) => data.scope
      | Advance(data) => data.scope
      | Close(data) => data.scope
      | Flush(_) => Scope0
    }

  pure def boundaryEvent(boundary: Boundary): EventId =
    match boundary {
      | Start(data) => data.event
      | Advance(data) => data.event
      | Close(data) => data.event
      | Flush(_) => Event0
    }

  pure def boundaryEventKey(boundary: Boundary): EventKey = {
    val scope = boundaryScope(boundary)
    {
      scopeId: scope,
      eventId: boundaryEvent(boundary),
    }
  }

  pure def isHook(boundary: Boundary): bool =
    match boundary {
      | Start(_) => true
      | Advance(_) => true
      | Close(_) => true
      | Flush(_) => false
    }

  pure def isStart(boundary: Boundary): bool =
    match boundary {
      | Start(_) => true
      | _ => false
    }

  pure def isAdvance(boundary: Boundary): bool =
    match boundary {
      | Advance(_) => true
      | _ => false
    }

  pure def isClose(boundary: Boundary): bool =
    match boundary {
      | Close(_) => true
      | _ => false
    }

  pure def isFlush(boundary: Boundary): bool =
    match boundary {
      | Flush(_) => true
      | _ => false
    }

  pure def singleScope(scopes: Set[ScopeId]): ScopeId =
    if (scopes.contains(Scope0)) {
      Scope0
    } else if (scopes.contains(Scope1)) {
      Scope1
    } else if (scopes.contains(Scope2)) {
      Scope2
    } else {
      Scope3
    }

  var worktrees: WorktreeId -> WorktreeState
  var scopes: ScopeId -> ScopeState
  var worktreeTrees: WorktreeId -> TreeId
  var processedEvents: Set[EventKey]
  var scopeStartCount: ScopeId -> int
  var everTerminal: Set[ScopeId]
  var attempts: AttemptId -> AttemptState
  var cursorHistory: Set[CursorCheckpoint]
  var evidenceAttempts: Set[AttemptId]
  var mutationEvents: Set[MutationEvent]

  def liveScopesOn(worktree: WorktreeId): Set[ScopeId] =
    SCOPES.filter(scope => {
      val state = scopes.get(scope)
      state.worktreeId == worktree and isLive(state.status)
    })

  def attributionFor(worktree: WorktreeId): Attribution = {
    val state = worktrees.get(worktree)
    val live = liveScopesOn(worktree)

    if (state.failureKind != Healthy or live.size() == 0) {
      IneligibleUnscoped
    } else if (live.size() == 1) {
      AiExclusive(singleScope(live))
    } else {
      AiContended
    }
  }

  pure def mkMutationEvent(
    worktree: WorktreeId,
    revision: int,
    beforeTree: TreeId,
    afterTree: TreeId,
    activeScopes: Set[ScopeId],
    tainted: bool,
    failureKind: FailureKind,
    attribution: Attribution,
    boundary: Boundary
  ): MutationEvent = {
    worktreeId: worktree,
    revision: revision,
    beforeTree: beforeTree,
    afterTree: afterTree,
    activeScopes: activeScopes,
    tainted: tainted,
    failureKind: failureKind,
    attribution: attribution,
    boundary: boundary,
  }

  action init: bool = all {
    worktrees' = WORKTREES.mapBy(_ => {
      cursorTree: Tree0,
      revision: 0,
      tainted: false,
      failureKind: Healthy,
    }),
    scopes' = SCOPES.mapBy(scope => {
      status: NeverSeen,
      actorKind: scopeActor(scope),
      worktreeId: scopeWorktree(scope),
    }),
    worktreeTrees' = WORKTREES.mapBy(_ => Tree0),
    processedEvents' = Set(),
    scopeStartCount' = SCOPES.mapBy(_ => 0),
    everTerminal' = Set(),
    attempts' = ATTEMPTS.mapBy(_ => {
      status: Available,
      boundary: Flush(WT0),
      expectedRevision: 0,
      beforeTree: Tree0,
      afterTree: Tree0,
    }),
    cursorHistory' = Set(
      { worktreeId: WT0, revision: 0, cursorTree: Tree0 },
      { worktreeId: WT1, revision: 0, cursorTree: Tree0 }
    ),
    evidenceAttempts' = Set(),
    mutationEvents' = Set(),
  }

  action stutter: bool = all {
    worktrees' = worktrees,
    scopes' = scopes,
    worktreeTrees' = worktreeTrees,
    processedEvents' = processedEvents,
    scopeStartCount' = scopeStartCount,
    everTerminal' = everTerminal,
    attempts' = attempts,
    cursorHistory' = cursorHistory,
    evidenceAttempts' = evidenceAttempts,
    mutationEvents' = mutationEvents,
  }

  action mutate(worktree: WorktreeId, newTree: TreeId): bool = all {
    worktrees' = worktrees,
    scopes' = scopes,
    worktreeTrees' = worktreeTrees.set(worktree, newTree),
    processedEvents' = processedEvents,
    scopeStartCount' = scopeStartCount,
    everTerminal' = everTerminal,
    attempts' = attempts,
    cursorHistory' = cursorHistory,
    evidenceAttempts' = evidenceAttempts,
    mutationEvents' = mutationEvents,
  }

  action prepare(attempt: AttemptId, boundary: Boundary): bool = {
    val oldAttempt = attempts.get(attempt)

    if (oldAttempt.status != Available) {
      stutter
    } else {
      val worktree = boundaryWorktree(boundary)
      val state = worktrees.get(worktree)
      all {
        worktrees' = worktrees,
        scopes' = scopes,
        worktreeTrees' = worktreeTrees,
        processedEvents' = processedEvents,
        scopeStartCount' = scopeStartCount,
        everTerminal' = everTerminal,
        attempts' = attempts.set(attempt, {
          status: Prepared,
          boundary: boundary,
          expectedRevision: state.revision,
          beforeTree: state.cursorTree,
          afterTree: worktreeTrees.get(worktree),
        }),
        cursorHistory' = cursorHistory,
        evidenceAttempts' = evidenceAttempts,
        mutationEvents' = mutationEvents,
      }
    }
  }

  action commitAttempt(attempt: AttemptId): bool = {
    val planned = attempts.get(attempt)
    val boundary = planned.boundary
    val worktree = boundaryWorktree(boundary)
    val state = worktrees.get(worktree)
    val scope = boundaryScope(boundary)
    val currentScope = scopes.get(scope)
    val event = boundaryEvent(boundary)
    val fresh =
      planned.status == Prepared and
      planned.expectedRevision == state.revision and
      planned.beforeTree == state.cursorTree and
      (not(isHook(boundary)) or
        not(processedEvents.contains(boundaryEventKey(boundary))))
    val observes =
      if (isStart(boundary)) {
        currentScope.status == NeverSeen
      } else if (isAdvance(boundary)) {
        isLive(currentScope.status)
      } else if (isClose(boundary)) {
        currentScope.status == NeverSeen or isLive(currentScope.status)
      } else {
        true
      }
    val accepted = fresh
    val changed = accepted and observes and
      planned.beforeTree != planned.afterTree
    val advancesRevision = accepted and (not(isFlush(boundary)) or changed)
    val live = liveScopesOn(worktree)
    val attribution = attributionFor(worktree)
    val emitted: Set[MutationEvent] =
      if (changed) {
        Set(mkMutationEvent(
          worktree,
          state.revision + 1,
          planned.beforeTree,
          planned.afterTree,
          live,
          state.tainted,
          state.failureKind,
          attribution,
          boundary
        ))
      } else {
        Set()
      }

    if (not(accepted)) {
      all {
        worktrees' = worktrees,
        scopes' = scopes,
        worktreeTrees' = worktreeTrees,
        processedEvents' = processedEvents,
        scopeStartCount' = scopeStartCount,
        everTerminal' = everTerminal,
        attempts' = attempts.set(attempt, {
          status: if (planned.status == Prepared) Rejected else planned.status,
          boundary: planned.boundary,
          expectedRevision: planned.expectedRevision,
          beforeTree: planned.beforeTree,
          afterTree: planned.afterTree,
        }),
        cursorHistory' = cursorHistory,
        evidenceAttempts' = evidenceAttempts,
        mutationEvents' = mutationEvents,
      }
    } else {
      val nextScope =
        if (isStart(boundary) and currentScope.status == NeverSeen) {
          scopes.set(scope, {
            status: Active,
            actorKind: currentScope.actorKind,
            worktreeId: worktree,
          })
        } else if (
          isClose(boundary) and
          (currentScope.status == NeverSeen or isLive(currentScope.status))
        ) {
          scopes.set(scope, {
            status: Closed,
            actorKind: currentScope.actorKind,
            worktreeId: worktree,
          })
        } else {
          scopes
        }
      val nextCursor = if (observes) planned.afterTree else state.cursorTree
      val nextWorktrees =
        if (advancesRevision) {
          worktrees.set(worktree, {
            cursorTree: nextCursor,
            revision: state.revision + 1,
            tainted: state.tainted,
            failureKind: state.failureKind,
          })
        } else {
          worktrees
        }
      val nextHistory =
        if (advancesRevision) {
          cursorHistory.union(Set({
            worktreeId: worktree,
            revision: state.revision + 1,
            cursorTree: nextCursor,
          }))
        } else {
          cursorHistory
        }
      val nextEvidenceAttempts =
        if (changed) evidenceAttempts.union(Set(attempt)) else evidenceAttempts
      val nextProcessed =
        if (isHook(boundary)) {
          processedEvents.union(Set(boundaryEventKey(boundary)))
        } else {
          processedEvents
        }
      val nextStartCount =
        if (isStart(boundary) and currentScope.status == NeverSeen) {
          scopeStartCount.set(scope, scopeStartCount.get(scope) + 1)
        } else {
          scopeStartCount
        }
      val nextTerminal =
        if (isClose(boundary) and
          (currentScope.status == NeverSeen or isLive(currentScope.status))) {
          everTerminal.union(Set(scope))
        } else {
          everTerminal
        }

      all {
        worktrees' = nextWorktrees,
        scopes' = nextScope,
        worktreeTrees' = worktreeTrees,
        processedEvents' = nextProcessed,
        scopeStartCount' = nextStartCount,
        everTerminal' = nextTerminal,
        attempts' = attempts.set(attempt, {
          status: Committed,
          boundary: planned.boundary,
          expectedRevision: planned.expectedRevision,
          beforeTree: planned.beforeTree,
          afterTree: planned.afterTree,
        }),
        cursorHistory' = nextHistory,
        evidenceAttempts' = nextEvidenceAttempts,
        mutationEvents' = mutationEvents.union(emitted),
      }
    }
  }

  action taint(worktree: WorktreeId): bool = {
    val state = worktrees.get(worktree)

    if (state.tainted) {
      stutter
    } else {
      all {
        worktrees' = worktrees.set(worktree, {
          cursorTree: state.cursorTree,
          revision: state.revision + 1,
          tainted: true,
          failureKind: SnapshotFailure,
        }),
        scopes' = scopes,
        worktreeTrees' = worktreeTrees,
        processedEvents' = processedEvents,
        scopeStartCount' = scopeStartCount,
        everTerminal' = everTerminal,
        attempts' = attempts,
        cursorHistory' = cursorHistory.union(Set({
          worktreeId: worktree,
          revision: state.revision + 1,
          cursorTree: state.cursorTree,
        })),
        evidenceAttempts' = evidenceAttempts,
        mutationEvents' = mutationEvents,
      }
    }
  }

  action databaseFailure(worktree: WorktreeId): bool = {
    val state = worktrees.get(worktree)

    if (state.tainted) {
      stutter
    } else {
      all {
        worktrees' = worktrees.set(worktree, {
          cursorTree: state.cursorTree,
          revision: state.revision + 1,
          tainted: true,
          failureKind: DatabaseFailure,
        }),
        scopes' = scopes,
        worktreeTrees' = worktreeTrees,
        processedEvents' = processedEvents,
        scopeStartCount' = scopeStartCount,
        everTerminal' = everTerminal,
        attempts' = attempts,
        cursorHistory' = cursorHistory.union(Set({
          worktreeId: worktree,
          revision: state.revision + 1,
          cursorTree: state.cursorTree,
        })),
        evidenceAttempts' = evidenceAttempts,
        mutationEvents' = mutationEvents,
      }
    }
  }

  action recover(worktree: WorktreeId): bool = {
    val state = worktrees.get(worktree)

    if (not(state.tainted)) {
      stutter
    } else {
      all {
        worktrees' = worktrees.set(worktree, {
          cursorTree: worktreeTrees.get(worktree),
          revision: state.revision + 1,
          tainted: false,
          failureKind: Healthy,
        }),
        scopes' = SCOPES.mapBy(scopeId => {
          val oldScope = scopes.get(scopeId)
          if (
            oldScope.worktreeId == worktree and
            isLive(oldScope.status)
          ) {
            {
              status: Closed,
              actorKind: oldScope.actorKind,
              worktreeId: oldScope.worktreeId,
            }
          } else {
            oldScope
          }
        }),
        worktreeTrees' = worktreeTrees,
        processedEvents' = processedEvents,
        scopeStartCount' = scopeStartCount,
        everTerminal' = everTerminal.union(liveScopesOn(worktree)),
        attempts' = attempts,
        cursorHistory' = cursorHistory.union(Set({
          worktreeId: worktree,
          revision: state.revision + 1,
          cursorTree: worktreeTrees.get(worktree),
        })),
        evidenceAttempts' = evidenceAttempts,
        mutationEvents' = mutationEvents,
      }
    }
  }

  action randomMutate: bool = {
    nondet worktree = WORKTREES.oneOf()
    nondet tree = TREES.oneOf()
    mutate(worktree, tree)
  }

  action randomPrepare: bool = {
    nondet attempt = ATTEMPTS.oneOf()
    nondet scope = SCOPES.oneOf()
    nondet event = EVENTS.oneOf()
    any {
      prepare(attempt, Start({ scope: scope, event: event })),
      prepare(attempt, Advance({ scope: scope, event: event })),
      prepare(attempt, Close({ scope: scope, event: event })),
      prepare(attempt, Flush(WT0)),
      prepare(attempt, Flush(WT1)),
    }
  }

  action randomCommit: bool = {
    nondet attempt = ATTEMPTS.oneOf()
    commitAttempt(attempt)
  }

  action randomTaint: bool = {
    nondet worktree = WORKTREES.oneOf()
    taint(worktree)
  }

  action randomRecover: bool = {
    nondet worktree = WORKTREES.oneOf()
    recover(worktree)
  }

  action randomDatabaseFailure: bool = {
    nondet worktree = WORKTREES.oneOf()
    databaseFailure(worktree)
  }

  action step: bool = any {
    randomMutate,
    randomPrepare,
    randomCommit,
    randomTaint,
    randomRecover,
    randomDatabaseFailure,
    stutter,
  }

  val CursorRevisionConsistent = WORKTREES.forall(worktree => {
    worktrees.get(worktree).revision >= 0
  })

  val FailureKindMatchesTaint = WORKTREES.forall(worktree => {
    worktrees.get(worktree).tainted ==
      (worktrees.get(worktree).failureKind != Healthy)
  })

  val TerminalScopesStayTerminal = everTerminal.forall(scope => {
    isTerminal(scopes.get(scope).status)
  })

  val ScopeActorIdentityIsStable = SCOPES.forall(scope => {
    scopes.get(scope).actorKind == scopeActor(scope)
  })

  val ScopeStartedAtMostOnce = SCOPES.forall(scope => {
    scopeStartCount.get(scope) >= 0 and scopeStartCount.get(scope) <= 1
  })

  val NoNoopMutationEvents = mutationEvents.forall(event => {
    event.beforeTree != event.afterTree
  })

  val MutationEventsHavePositiveRevision = mutationEvents.forall(event => {
    event.revision > 0
  })

  val MutationEventUniquePerWorktreeRevision = mutationEvents.forall(left => {
    mutationEvents.forall(right => {
      (left.worktreeId == right.worktreeId and left.revision == right.revision)
      implies left == right
    })
  })

  val CursorHistoryUniquePerWorktreeRevision = cursorHistory.forall(left => {
    cursorHistory.forall(right => {
      (left.worktreeId == right.worktreeId and left.revision == right.revision)
      implies left == right
    })
  })

  val CursorHistoryHasCurrentState = WORKTREES.forall(worktree => {
    cursorHistory.exists(checkpoint =>
      checkpoint.worktreeId == worktree and
      checkpoint.revision == worktrees.get(worktree).revision and
      checkpoint.cursorTree == worktrees.get(worktree).cursorTree
    )
  })

  val MutationEventsMatchCursorHistory = mutationEvents.forall(event => {
    cursorHistory.exists(before =>
      before.worktreeId == event.worktreeId and
      before.revision == event.revision - 1 and
      before.cursorTree == event.beforeTree
    ) and
    cursorHistory.exists(after =>
      after.worktreeId == event.worktreeId and
      after.revision == event.revision and
      after.cursorTree == event.afterTree
    )
  })

  val MutationFailureKindMatchesTaint = mutationEvents.forall(event => {
    event.tainted == (event.failureKind != Healthy)
  })

  val AttributionMatchesObservedScopes = mutationEvents.forall(event => {
    if (event.failureKind != Healthy) {
      event.attribution == IneligibleUnscoped
    } else if (event.activeScopes.size() == 0) {
      event.attribution == IneligibleUnscoped
    } else if (event.activeScopes.size() == 1) {
      match event.attribution {
        | AiExclusive(scope) => event.activeScopes.contains(scope)
        | _ => false
      }
    } else {
      event.attribution == AiContended
    }
  })

  val ExclusiveAttributionRequiresOneScope = mutationEvents.forall(event => {
    match event.attribution {
      | AiExclusive(scope) =>
        event.failureKind == Healthy and
        event.activeScopes.size() == 1 and
        event.activeScopes.contains(scope)
      | _ => true
    }
  })

  val StaleAttemptsDoNotCommitEvidence = ATTEMPTS.forall(id => {
    attempts.get(id).status != Rejected or not(evidenceAttempts.contains(id))
  })

  val HasExclusiveEvidence = mutationEvents.exists(event => {
    match event.attribution {
      | AiExclusive(_) => true
      | _ => false
    }
  })

  val HasContendedEvidence = mutationEvents.exists(event => {
    event.attribution == AiContended
  })

  val HasUnscopedEvidence = mutationEvents.exists(event => {
    event.attribution == IneligibleUnscoped
  })

  val HasRejectedAttempt = ATTEMPTS.exists(id => {
    attempts.get(id).status == Rejected
  })

  val Safety = and {
    CursorRevisionConsistent,
    FailureKindMatchesTaint,
    TerminalScopesStayTerminal,
    ScopeActorIdentityIsStable,
    ScopeStartedAtMostOnce,
    NoNoopMutationEvents,
    MutationEventsHavePositiveRevision,
    MutationEventUniquePerWorktreeRevision,
    CursorHistoryUniquePerWorktreeRevision,
    CursorHistoryHasCurrentState,
    MutationEventsMatchCursorHistory,
    MutationFailureKindMatchesTaint,
    AttributionMatchesObservedScopes,
    ExclusiveAttributionRequiresOneScope,
    StaleAttemptsDoNotCommitEvidence,
  }

  run testStartObservesBeforeActivation =
    init
      .then(mutate(WT0, Tree1))
      .then(prepare(Attempt0, Start({ scope: Scope0, event: Event0 })))
      .then(commitAttempt(Attempt0))
      .expect(worktrees.get(WT0).cursorTree == Tree1)
      .expect(scopes.get(Scope0).status == Active)
      .expect(mutationEvents.size() == 1)
      .expect(
        mutationEvents.forall(event =>
          event.attribution == IneligibleUnscoped
        )
      )
      .expect(Safety)

  run testCloseObservesBeforeDeactivation =
    init
      .then(prepare(Attempt0, Start({ scope: Scope0, event: Event0 })))
      .then(commitAttempt(Attempt0))
      .then(mutate(WT0, Tree1))
      .then(prepare(Attempt1, Close({ scope: Scope0, event: Event1 })))
      .then(commitAttempt(Attempt1))
      .expect(scopes.get(Scope0).status == Closed)
      .expect(worktrees.get(WT0).cursorTree == Tree1)
      .expect(
        mutationEvents.exists(event =>
          event.attribution == AiExclusive(Scope0)
        )
      )
      .expect(Safety)

  run testContendedIntervalsRemainAiContended =
    init
      .then(prepare(Attempt0, Start({ scope: Scope0, event: Event0 })))
      .then(commitAttempt(Attempt0))
      .then(prepare(Attempt1, Start({ scope: Scope2, event: Event1 })))
      .then(commitAttempt(Attempt1))
      .then(mutate(WT0, Tree1))
      .then(prepare(Attempt2, Flush(WT0)))
      .then(commitAttempt(Attempt2))
      .expect(
        mutationEvents.exists(event =>
          event.afterTree == Tree1 and
          event.attribution == AiContended
        )
      )
      .expect(Safety)

  run testNoChangeHookReplayCannotStealFutureChange =
    init
      .then(prepare(Attempt0, Start({ scope: Scope0, event: Event0 })))
      .then(commitAttempt(Attempt0))
      .then(prepare(Attempt1, Advance({ scope: Scope0, event: Event1 })))
      .then(commitAttempt(Attempt1))
      .then(mutate(WT0, Tree1))
      .then(prepare(Attempt2, Advance({ scope: Scope0, event: Event1 })))
      .then(commitAttempt(Attempt2))
      .expect(worktrees.get(WT0).cursorTree == Tree0)
      .expect(mutationEvents.size() == 0)
      .expect(processedEvents.contains(boundaryEventKey(Advance({
        scope: Scope0,
        event: Event1,
      }))))
      .expect(Safety)

  run testConcurrentObservationsHaveOneWinner =
    init
      .then(prepare(Attempt0, Start({ scope: Scope0, event: Event0 })))
      .then(commitAttempt(Attempt0))
      .then(mutate(WT0, Tree1))
      .then(prepare(Attempt1, Advance({ scope: Scope0, event: Event1 })))
      .then(prepare(Attempt2, Advance({ scope: Scope0, event: Event2 })))
      .then(commitAttempt(Attempt1))
      .then(commitAttempt(Attempt2))
      .expect(worktrees.get(WT0).cursorTree == Tree1)
      .expect(mutationEvents.size() == 1)
      .expect(attempts.get(Attempt2).status == Rejected)
      .expect(Safety)

  run testScopeChangeInvalidatesExclusivePlan =
    init
      .then(prepare(Attempt0, Start({ scope: Scope0, event: Event0 })))
      .then(commitAttempt(Attempt0))
      .then(mutate(WT0, Tree1))
      .then(prepare(Attempt1, Advance({ scope: Scope0, event: Event1 })))
      .then(prepare(Attempt2, Start({ scope: Scope2, event: Event2 })))
      .then(commitAttempt(Attempt2))
      .then(commitAttempt(Attempt1))
      .expect(attempts.get(Attempt1).status == Rejected)
      .expect(
        mutationEvents.exists(event =>
          event.beforeTree == Tree0 and
          event.afterTree == Tree1 and
          event.attribution == AiExclusive(Scope0)
        )
      )
      .expect(mutationEvents.size() == 1)
      .expect(Safety)

  run testTaintOnlyWeakensAttribution =
    init
      .then(prepare(Attempt0, Start({ scope: Scope0, event: Event0 })))
      .then(commitAttempt(Attempt0))
      .then(taint(WT0))
      .then(mutate(WT0, Tree1))
      .then(prepare(Attempt1, Advance({ scope: Scope0, event: Event1 })))
      .then(commitAttempt(Attempt1))
      .expect(
        mutationEvents.exists(event =>
          event.afterTree == Tree1 and
          event.attribution == IneligibleUnscoped
        )
      )
      .expect(Safety)

  run testRecoveryEstablishesBaseline =
    init
      .then(taint(WT0))
      .then(mutate(WT0, Tree1))
      .then(recover(WT0))
      .expect(worktrees.get(WT0).cursorTree == Tree1)
      .expect(not(worktrees.get(WT0).tainted))
      .expect(mutationEvents.size() == 0)
      .expect(Safety)

  run testContentionDoesNotUpgradeAfterRecovery =
    init
      .then(prepare(Attempt0, Start({ scope: Scope0, event: Event0 })))
      .then(commitAttempt(Attempt0))
      .then(prepare(Attempt1, Start({ scope: Scope2, event: Event1 })))
      .then(commitAttempt(Attempt1))
      .then(mutate(WT0, Tree1))
      .then(prepare(Attempt2, Flush(WT0)))
      .then(commitAttempt(Attempt2))
      .then(prepare(Attempt3, Close({ scope: Scope2, event: Event2 })))
      .then(commitAttempt(Attempt3))
      .then(mutate(WT0, Tree2))
      .then(prepare(Attempt4, Close({ scope: Scope0, event: Event3 })))
      .then(commitAttempt(Attempt4))
      .expect(
        mutationEvents.exists(event =>
          event.afterTree == Tree1 and
          event.attribution == AiContended
        )
      )
      .expect(
        mutationEvents.exists(event =>
          event.afterTree == Tree2 and
          event.attribution == AiExclusive(Scope0)
        )
      )
      .expect(Safety)

  run testTaintInvalidatesPreparedObservation =
    init
      .then(prepare(Attempt0, Start({ scope: Scope0, event: Event0 })))
      .then(commitAttempt(Attempt0))
      .then(mutate(WT0, Tree1))
      .then(prepare(Attempt1, Advance({ scope: Scope0, event: Event1 })))
      .then(taint(WT0))
      .then(commitAttempt(Attempt1))
      .expect(attempts.get(Attempt1).status == Rejected)
      .expect(worktrees.get(WT0).cursorTree == Tree0)
      .expect(mutationEvents.size() == 0)
      .expect(Safety)

  run testClosedScopeCannotReactivate =
    init
      .then(prepare(Attempt0, Start({ scope: Scope0, event: Event0 })))
      .then(commitAttempt(Attempt0))
      .then(prepare(Attempt1, Close({ scope: Scope0, event: Event1 })))
      .then(commitAttempt(Attempt1))
      .then(mutate(WT0, Tree1))
      .then(prepare(Attempt2, Start({ scope: Scope0, event: Event2 })))
      .then(commitAttempt(Attempt2))
      .expect(scopes.get(Scope0).status == Closed)
      .expect(worktrees.get(WT0).cursorTree == Tree0)
      .expect(mutationEvents.size() == 0)
      .expect(Safety)

  run testRecoveryInvalidatesActiveScopes =
    init
      .then(prepare(Attempt0, Start({ scope: Scope0, event: Event0 })))
      .then(commitAttempt(Attempt0))
      .then(taint(WT0))
      .then(mutate(WT0, Tree1))
      .then(recover(WT0))
      .expect(scopes.get(Scope0).status == Closed)
      .expect(worktrees.get(WT0).cursorTree == Tree1)
      .expect(mutationEvents.size() == 0)
      .then(prepare(Attempt1, Start({ scope: Scope1, event: Event1 })))
      .then(commitAttempt(Attempt1))
      .then(mutate(WT0, Tree2))
      .then(prepare(Attempt2, Advance({ scope: Scope1, event: Event2 })))
      .then(commitAttempt(Attempt2))
      .expect(
        mutationEvents.exists(event =>
          event.afterTree == Tree2 and
          event.attribution == AiExclusive(Scope1)
        )
      )
      .expect(Safety)

  run testMultipleActiveScopesRemainContended =
    init
      .then(prepare(Attempt0, Start({ scope: Scope0, event: Event0 })))
      .then(commitAttempt(Attempt0))
      .then(prepare(Attempt1, Start({ scope: Scope1, event: Event1 })))
      .then(commitAttempt(Attempt1))
      .then(prepare(Attempt2, Start({ scope: Scope2, event: Event2 })))
      .then(commitAttempt(Attempt2))
      .then(mutate(WT0, Tree1))
      .then(prepare(Attempt3, Flush(WT0)))
      .then(commitAttempt(Attempt3))
      .then(prepare(Attempt4, Close({ scope: Scope2, event: Event3 })))
      .then(commitAttempt(Attempt4))
      .then(mutate(WT0, Tree2))
      .then(prepare(Attempt5, Flush(WT0)))
      .then(commitAttempt(Attempt5))
      .expect(scopes.get(Scope0).status == Active)
      .expect(scopes.get(Scope1).status == Active)
      .expect(scopes.get(Scope2).status == Closed)
      .expect(
        mutationEvents.forall(event =>
          event.attribution == AiContended
        )
      )
      .expect(Safety)

  run testDatabaseFailureWeakensAttribution =
    init
      .then(prepare(Attempt0, Start({ scope: Scope0, event: Event0 })))
      .then(commitAttempt(Attempt0))
      .then(databaseFailure(WT0))
      .then(mutate(WT0, Tree1))
      .then(prepare(Attempt1, Advance({ scope: Scope0, event: Event1 })))
      .then(commitAttempt(Attempt1))
      .expect(
        mutationEvents.exists(event =>
          event.afterTree == Tree1 and
          event.failureKind == DatabaseFailure and
          event.attribution == IneligibleUnscoped
        )
      )
      .expect(Safety)

  run testDifferentWorktreesAreIndependent =
    init
      .then(prepare(Attempt0, Start({ scope: Scope0, event: Event0 })))
      .then(commitAttempt(Attempt0))
      .then(prepare(Attempt1, Start({ scope: Scope3, event: Event1 })))
      .then(commitAttempt(Attempt1))
      .then(mutate(WT0, Tree1))
      .then(prepare(Attempt2, Advance({ scope: Scope0, event: Event2 })))
      .then(commitAttempt(Attempt2))
      .expect(scopes.get(Scope0).status == Active)
      .expect(scopes.get(Scope3).status == Active)
      .expect(worktrees.get(WT1).cursorTree == Tree0)
      .expect(Safety)
}
