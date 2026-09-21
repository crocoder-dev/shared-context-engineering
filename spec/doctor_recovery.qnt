module doctor_recovery {
  type AttemptId = Attempt0 | Attempt1 | Attempt2

  type AttemptPhase = NotAllocated | PendingStart | Active | PendingAbandon | Removed

  type RecoveryState = Clear | Pending | Flushing

  type Health = Healthy | Recovering | Blocked | Invalid

  type OwnerReading = SeenAlive | SeenDead | SeenUnknown

  type StateTxnHolder = StateTxnFree | DoctorStateTxn

  type FixState = NotAttempted | RepairAttempted | RepairCompleted | ReportedFixed | ReportedManual

  val ATTEMPTS: Set[AttemptId] = Set(Attempt0, Attempt1, Attempt2)

  pure def soundReadings(alive: bool): Set[OwnerReading] =
    if (alive) Set(SeenAlive, SeenUnknown) else Set(SeenAlive, SeenDead, SeenUnknown)

  pure def adapterHealth(ph: AttemptId -> AttemptPhase, rec: RecoveryState): Health = {
    val hasPendingStart = ATTEMPTS.exists(a => ph.get(a) == PendingStart)
    val hasPendingAbandon = ATTEMPTS.exists(a => ph.get(a) == PendingAbandon)
    if (rec == Clear and hasPendingAbandon) Invalid
    else if (hasPendingStart) Blocked
    else if (rec == Clear) Healthy
    else Recovering
  }

  pure def abandonTransition(
    ph: AttemptId -> AttemptPhase,
    wasPendingAbandon: Set[AttemptId],
    rec: RecoveryState,
    attempt: AttemptId
  ): { phase: AttemptId -> AttemptPhase, recovery: RecoveryState, everWasPendingAbandon: Set[AttemptId] } = {
    phase: ph.set(attempt, PendingAbandon),
    recovery: if (rec == Clear) Pending else rec,
    everWasPendingAbandon: wasPendingAbandon.union(Set(attempt)),
  }

  var phase: AttemptId -> AttemptPhase
  var recovery: RecoveryState
  var ownerAlive: AttemptId -> bool
  var everRemoved: Set[AttemptId]
  var everWasPendingAbandon: Set[AttemptId]
  var pendingAbandonFromDoctor: Set[AttemptId]
  var stateTxn: StateTxnHolder
  var lastDiagnosis: AttemptId -> OwnerReading
  var fixState: FixState

  action init: bool = all {
    phase' = ATTEMPTS.mapBy(_ => NotAllocated),
    recovery' = Clear,
    ownerAlive' = ATTEMPTS.mapBy(_ => false),
    everRemoved' = Set(),
    everWasPendingAbandon' = Set(),
    pendingAbandonFromDoctor' = Set(),
    stateTxn' = StateTxnFree,
    lastDiagnosis' = ATTEMPTS.mapBy(_ => SeenUnknown),
    fixState' = NotAttempted,
  }

  action stutter: bool = all {
    phase' = phase,
    recovery' = recovery,
    ownerAlive' = ownerAlive,
    everRemoved' = everRemoved,
    everWasPendingAbandon' = everWasPendingAbandon,
    pendingAbandonFromDoctor' = pendingAbandonFromDoctor,
    stateTxn' = stateTxn,
    lastDiagnosis' = lastDiagnosis,
    fixState' = fixState,
  }

  action hookAllocate(attempt: AttemptId): bool = all {
    phase.get(attempt) == NotAllocated,
    stateTxn == StateTxnFree,
    phase' = phase.set(attempt, PendingStart),
    recovery' = recovery,
    ownerAlive' = ownerAlive.set(attempt, true),
    everRemoved' = everRemoved,
    everWasPendingAbandon' = everWasPendingAbandon,
    pendingAbandonFromDoctor' = pendingAbandonFromDoctor,
    stateTxn' = stateTxn,
    lastDiagnosis' = lastDiagnosis.set(attempt, SeenUnknown),
    fixState' = NotAttempted,
  }

  action hookAdvance(attempt: AttemptId): bool = all {
    phase.get(attempt) == PendingStart,
    stateTxn == StateTxnFree,
    phase' = phase.set(attempt, Active),
    recovery' = recovery,
    ownerAlive' = ownerAlive,
    everRemoved' = everRemoved,
    everWasPendingAbandon' = everWasPendingAbandon,
    pendingAbandonFromDoctor' = pendingAbandonFromDoctor,
    stateTxn' = stateTxn,
    lastDiagnosis' = lastDiagnosis,
    fixState' = fixState,
  }

  action hookDecideAbandon(attempt: AttemptId): bool = {
    val current = phase.get(attempt)
    val t = abandonTransition(phase, everWasPendingAbandon, recovery, attempt)
    all {
      current == PendingStart or current == Active,
      stateTxn == StateTxnFree,
      phase' = t.phase,
      recovery' = t.recovery,
      everWasPendingAbandon' = t.everWasPendingAbandon,
      ownerAlive' = ownerAlive,
      everRemoved' = everRemoved,
      pendingAbandonFromDoctor' = pendingAbandonFromDoctor,
      stateTxn' = stateTxn,
      lastDiagnosis' = lastDiagnosis,
      fixState' = fixState,
    }
  }

  action hookOwnerCrashes(attempt: AttemptId): bool = all {
    phase.get(attempt) != NotAllocated,
    phase.get(attempt) != Removed,
    ownerAlive.get(attempt),
    phase' = phase,
    recovery' = recovery,
    ownerAlive' = ownerAlive.set(attempt, false),
    everRemoved' = everRemoved,
    everWasPendingAbandon' = everWasPendingAbandon,
    pendingAbandonFromDoctor' = pendingAbandonFromDoctor,
    stateTxn' = stateTxn,
    lastDiagnosis' = lastDiagnosis,
    fixState' = fixState,
  }

  action recoveryProgress(attempt: AttemptId): bool = all {
    phase.get(attempt) == PendingAbandon,
    recovery == Pending,
    stateTxn == StateTxnFree,
    phase' = phase,
    recovery' = Flushing,
    ownerAlive' = ownerAlive,
    everRemoved' = everRemoved,
    everWasPendingAbandon' = everWasPendingAbandon,
    pendingAbandonFromDoctor' = pendingAbandonFromDoctor,
    stateTxn' = stateTxn,
    lastDiagnosis' = lastDiagnosis,
    fixState' = fixState,
  }

  action completeAbandon(attempt: AttemptId): bool = {
    val newPhase = phase.set(attempt, Removed)
    val otherObligationsRemain = ATTEMPTS.exists(a => newPhase.get(a) == PendingAbandon)
    all {
      phase.get(attempt) == PendingAbandon,
      recovery == Flushing,
      stateTxn == StateTxnFree,
      phase' = newPhase,
      recovery' = if (otherObligationsRemain) Pending else Clear,
      ownerAlive' = ownerAlive,
      everRemoved' = everRemoved.union(Set(attempt)),
      everWasPendingAbandon' = everWasPendingAbandon,
      pendingAbandonFromDoctor' = pendingAbandonFromDoctor,
      stateTxn' = stateTxn,
      lastDiagnosis' = lastDiagnosis,
      fixState' =
        if (fixState == RepairAttempted) RepairCompleted else fixState,
    }
  }

  action doctorDiagnoseWith(attempt: AttemptId, reading: OwnerReading): bool = all {
    soundReadings(ownerAlive.get(attempt)).contains(reading),
    phase' = phase,
    recovery' = recovery,
    ownerAlive' = ownerAlive,
    everRemoved' = everRemoved,
    everWasPendingAbandon' = everWasPendingAbandon,
    pendingAbandonFromDoctor' = pendingAbandonFromDoctor,
    stateTxn' = stateTxn,
    lastDiagnosis' = lastDiagnosis.set(attempt, reading),
    fixState' = fixState,
  }

  action doctorDiagnose(attempt: AttemptId): bool = {
    nondet reading = soundReadings(ownerAlive.get(attempt)).oneOf()
    doctorDiagnoseWith(attempt, reading)
  }

  action doctorAcquireStateTxn: bool = all {
    stateTxn == StateTxnFree,
    phase' = phase,
    recovery' = recovery,
    ownerAlive' = ownerAlive,
    everRemoved' = everRemoved,
    everWasPendingAbandon' = everWasPendingAbandon,
    pendingAbandonFromDoctor' = pendingAbandonFromDoctor,
    stateTxn' = DoctorStateTxn,
    lastDiagnosis' = lastDiagnosis,
    fixState' = fixState,
  }

  action doctorReleaseStateTxn: bool = all {
    stateTxn == DoctorStateTxn,
    phase' = phase,
    recovery' = recovery,
    ownerAlive' = ownerAlive,
    everRemoved' = everRemoved,
    everWasPendingAbandon' = everWasPendingAbandon,
    pendingAbandonFromDoctor' = pendingAbandonFromDoctor,
    stateTxn' = StateTxnFree,
    lastDiagnosis' = lastDiagnosis,
    fixState' = fixState,
  }

  action doctorAttemptRepairWith(attempt: AttemptId, reading: OwnerReading): bool = {
    val eligible =
      stateTxn == DoctorStateTxn and phase.get(attempt) == PendingStart and reading == SeenDead
    val t = abandonTransition(phase, everWasPendingAbandon, recovery, attempt)
    all {
      phase.get(attempt) != NotAllocated,
      soundReadings(ownerAlive.get(attempt)).contains(reading),
      phase' = if (eligible) t.phase else phase,
      recovery' = if (eligible) t.recovery else recovery,
      everWasPendingAbandon' = if (eligible) t.everWasPendingAbandon else everWasPendingAbandon,
      ownerAlive' = ownerAlive,
      everRemoved' = everRemoved,
      pendingAbandonFromDoctor' =
        if (eligible) pendingAbandonFromDoctor.union(Set(attempt)) else pendingAbandonFromDoctor,
      stateTxn' = stateTxn,
      lastDiagnosis' = lastDiagnosis.set(attempt, reading),
      fixState' = RepairAttempted,
    }
  }

  action doctorAttemptRepair(attempt: AttemptId): bool = {
    nondet reading = soundReadings(ownerAlive.get(attempt)).oneOf()
    doctorAttemptRepairWith(attempt, reading)
  }

  action doctorReportResult: bool = {
    val attempted = fixState == RepairAttempted or fixState == RepairCompleted
    val h = adapterHealth(phase, recovery)
    val outcome = if (h == Healthy or h == Recovering) ReportedFixed else ReportedManual
    all {
      attempted,
      fixState' = outcome,
      phase' = phase,
      recovery' = recovery,
      ownerAlive' = ownerAlive,
      everRemoved' = everRemoved,
      everWasPendingAbandon' = everWasPendingAbandon,
      pendingAbandonFromDoctor' = pendingAbandonFromDoctor,
      stateTxn' = stateTxn,
      lastDiagnosis' = lastDiagnosis,
    }
  }

  action step: bool = any {
    { nondet a = ATTEMPTS.oneOf(); hookAllocate(a) },
    { nondet a = ATTEMPTS.oneOf(); hookAdvance(a) },
    { nondet a = ATTEMPTS.oneOf(); hookDecideAbandon(a) },
    { nondet a = ATTEMPTS.oneOf(); hookOwnerCrashes(a) },
    { nondet a = ATTEMPTS.oneOf(); recoveryProgress(a) },
    { nondet a = ATTEMPTS.oneOf(); completeAbandon(a) },
    { nondet a = ATTEMPTS.oneOf(); doctorDiagnose(a) },
    doctorAcquireStateTxn,
    doctorReleaseStateTxn,
    { nondet a = ATTEMPTS.oneOf(); doctorAttemptRepair(a) },
    doctorReportResult,
    stutter,
  }

  val DoctorNeverAbandonsALiveOwner = ATTEMPTS.forall(a =>
    pendingAbandonFromDoctor.contains(a) implies not(ownerAlive.get(a))
  )

  val UnknownOwnerNeverProvesDeath = ATTEMPTS.forall(a =>
    lastDiagnosis.get(a) == SeenDead implies not(ownerAlive.get(a))
  )

  val RecoveryNeverClearedWithUnresolvedAbandon =
    recovery == Clear implies ATTEMPTS.forall(a => phase.get(a) != PendingAbandon)

  val NoOrdinaryTransitionProducesInvalidHealth =
    adapterHealth(phase, recovery) != Invalid

  val DoctorRepairProducesOnlyOrdinaryLifecycleShapes = ATTEMPTS.forall(a =>
    pendingAbandonFromDoctor.contains(a) implies (
      phase.get(a) == Removed
        or (phase.get(a) == PendingAbandon and (recovery == Pending or recovery == Flushing))
    )
  )

  val RemovedAttemptsAreNeverResurrected =
    everRemoved.forall(a => phase.get(a) == Removed)

  val InterruptedRecoveryStaysInOrdinaryRetryableState = ATTEMPTS.forall(a =>
    (everWasPendingAbandon.contains(a) and not(everRemoved.contains(a)))
      implies (phase.get(a) == PendingAbandon and (recovery == Pending or recovery == Flushing))
  )

  val ReportedFixedExcludesBlockedOrInvalid =
    fixState == ReportedFixed implies
      (adapterHealth(phase, recovery) != Blocked and adapterHealth(phase, recovery) != Invalid)

  val Safety = and {
    DoctorNeverAbandonsALiveOwner,
    UnknownOwnerNeverProvesDeath,
    RecoveryNeverClearedWithUnresolvedAbandon,
    NoOrdinaryTransitionProducesInvalidHealth,
    DoctorRepairProducesOnlyOrdinaryLifecycleShapes,
    RemovedAttemptsAreNeverResurrected,
    InterruptedRecoveryStaysInOrdinaryRetryableState,
    ReportedFixedExcludesBlockedOrInvalid,
  }

  run testOrdinaryLifecycleReachesActive =
    init
      .then(hookAllocate(Attempt0))
      .expect(phase.get(Attempt0) == PendingStart)
      .expect(adapterHealth(phase, recovery) == Blocked)
      .then(hookAdvance(Attempt0))
      .expect(phase.get(Attempt0) == Active)
      .expect(adapterHealth(phase, recovery) == Healthy)
      .expect(Safety)

  run testHookOwnAbandonNeedsNoOwnerEvidence =
    init
      .then(hookAllocate(Attempt0))
      .then(hookDecideAbandon(Attempt0))
      .expect(phase.get(Attempt0) == PendingAbandon)
      .expect(recovery == Pending)
      .expect(adapterHealth(phase, recovery) == Recovering)
      .then(recoveryProgress(Attempt0))
      .expect(recovery == Flushing)
      .then(completeAbandon(Attempt0))
      .expect(phase.get(Attempt0) == Removed)
      .expect(recovery == Clear)
      .expect(adapterHealth(phase, recovery) == Healthy)
      .expect(Safety)

  run testMultiplePendingAbandonShareOneRecoveryPipeline =
    init
      .then(hookAllocate(Attempt0))
      .then(hookAllocate(Attempt1))
      .then(hookDecideAbandon(Attempt0))
      .expect(phase.get(Attempt0) == PendingAbandon)
      .expect(recovery == Pending)
      .then(hookDecideAbandon(Attempt1))
      .expect(phase.get(Attempt0) == PendingAbandon)
      .expect(phase.get(Attempt1) == PendingAbandon)
      .expect(recovery == Pending)
      .expect(adapterHealth(phase, recovery) == Recovering)
      .expect(Safety)
      .then(recoveryProgress(Attempt0))
      .expect(recovery == Flushing)
      .then(completeAbandon(Attempt0))
      .expect(phase.get(Attempt0) == Removed)
      .expect(phase.get(Attempt1) == PendingAbandon)
      .expect(recovery != Clear)
      .expect(adapterHealth(phase, recovery) == Recovering)
      .expect(Safety)
      .then(recoveryProgress(Attempt1))
      .expect(recovery == Flushing)
      .then(completeAbandon(Attempt1))
      .expect(phase.get(Attempt1) == Removed)
      .expect(recovery == Clear)
      .expect(adapterHealth(phase, recovery) == Healthy)
      .expect(Safety)

  run testDoctorRepairsADefinitelyDeadOwner =
    init
      .then(hookAllocate(Attempt0))
      .then(hookOwnerCrashes(Attempt0))
      .then(doctorDiagnoseWith(Attempt0, SeenDead))
      .then(doctorAcquireStateTxn)
      .then(doctorAttemptRepairWith(Attempt0, SeenDead))
      .expect(phase.get(Attempt0) == PendingAbandon)
      .expect(recovery == Pending)
      .expect(pendingAbandonFromDoctor.contains(Attempt0))
      .then(doctorReleaseStateTxn)
      .then(recoveryProgress(Attempt0))
      .then(completeAbandon(Attempt0))
      .expect(phase.get(Attempt0) == Removed)
      .expect(adapterHealth(phase, recovery) == Healthy)
      .then(doctorReportResult)
      .expect(fixState == ReportedFixed)
      .expect(Safety)

  run testDoctorRefusesALiveOwner =
    init
      .then(hookAllocate(Attempt0))
      .then(doctorDiagnose(Attempt0))
      .then(doctorAcquireStateTxn)
      .then(doctorAttemptRepair(Attempt0))
      .expect(phase.get(Attempt0) == PendingStart)
      .expect(not(pendingAbandonFromDoctor.contains(Attempt0)))
      .then(doctorReportResult)
      .expect(fixState == ReportedManual)
      .expect(Safety)

  run testUnknownOwnerRepairIsRefused =
    init
      .then(hookAllocate(Attempt0))
      .then(doctorAcquireStateTxn)
      .then(doctorAttemptRepairWith(Attempt0, SeenUnknown))
      .expect(phase.get(Attempt0) == PendingStart)
      .expect(not(pendingAbandonFromDoctor.contains(Attempt0)))
      .expect(adapterHealth(phase, recovery) == Blocked)
      .then(doctorReportResult)
      .expect(fixState == ReportedManual)
      .expect(Safety)

  run testStaleDiagnosisRefusedAfterConcurrentPhaseChange =
    init
      .then(hookAllocate(Attempt0))
      .then(hookOwnerCrashes(Attempt0))
      .then(doctorDiagnoseWith(Attempt0, SeenDead))
      .expect(lastDiagnosis.get(Attempt0) == SeenDead)
      .then(hookAdvance(Attempt0))
      .expect(phase.get(Attempt0) == Active)
      .then(doctorAcquireStateTxn)
      .then(doctorAttemptRepairWith(Attempt0, SeenDead))
      .expect(phase.get(Attempt0) == Active)
      .expect(not(pendingAbandonFromDoctor.contains(Attempt0)))
      .expect(Safety)

  run testInterruptedRepairIsRetryable =
    init
      .then(hookAllocate(Attempt0))
      .then(hookOwnerCrashes(Attempt0))
      .then(doctorDiagnoseWith(Attempt0, SeenDead))
      .then(doctorAcquireStateTxn)
      .then(doctorAttemptRepairWith(Attempt0, SeenDead))
      .then(doctorReleaseStateTxn)
      .expect(phase.get(Attempt0) == PendingAbandon)
      .expect(recovery == Pending)
      .expect(adapterHealth(phase, recovery) == Recovering)
      .then(stutter)
      .expect(phase.get(Attempt0) == PendingAbandon)
      .expect(recovery == Pending)
      .expect(InterruptedRecoveryStaysInOrdinaryRetryableState)
      .then(recoveryProgress(Attempt0))
      .expect(recovery == Flushing)
      .then(stutter)
      .expect(phase.get(Attempt0) == PendingAbandon)
      .expect(recovery == Flushing)
      .expect(InterruptedRecoveryStaysInOrdinaryRetryableState)
      .then(completeAbandon(Attempt0))
      .expect(phase.get(Attempt0) == Removed)
      .expect(recovery == Clear)
      .expect(everRemoved.contains(Attempt0))
      .expect(Safety)

  run testSuccessfulRepairReachesRecoveringThenHealthy =
    init
      .then(hookAllocate(Attempt0))
      .then(hookOwnerCrashes(Attempt0))
      .then(doctorDiagnoseWith(Attempt0, SeenDead))
      .then(doctorAcquireStateTxn)
      .then(doctorAttemptRepairWith(Attempt0, SeenDead))
      .then(doctorReleaseStateTxn)
      .expect(adapterHealth(phase, recovery) == Recovering)
      .then(doctorReportResult)
      .expect(fixState == ReportedFixed)
      .then(recoveryProgress(Attempt0))
      .then(completeAbandon(Attempt0))
      .expect(adapterHealth(phase, recovery) == Healthy)
      .expect(Safety)

  run testRepairingOneDeadBlockerLeavesOtherBlockerBlocked =
    init
      .then(hookAllocate(Attempt0))
      .then(hookAllocate(Attempt1))
      .then(hookOwnerCrashes(Attempt0))
      .expect(adapterHealth(phase, recovery) == Blocked)
      .then(doctorDiagnoseWith(Attempt0, SeenDead))
      .then(doctorDiagnoseWith(Attempt1, SeenUnknown))
      .then(doctorAcquireStateTxn)
      .then(doctorAttemptRepairWith(Attempt0, SeenDead))
      .expect(phase.get(Attempt0) == PendingAbandon)
      .expect(phase.get(Attempt1) == PendingStart)
      .then(doctorReleaseStateTxn)
      .expect(adapterHealth(phase, recovery) == Blocked)
      .then(doctorReportResult)
      .expect(fixState == ReportedManual)
      .expect(not(fixState == ReportedFixed))
      .then(recoveryProgress(Attempt0))
      .then(completeAbandon(Attempt0))
      .expect(phase.get(Attempt0) == Removed)
      .expect(phase.get(Attempt1) == PendingStart)
      .expect(adapterHealth(phase, recovery) == Blocked)
      .expect(not(fixState == ReportedFixed))
      .expect(Safety)

  run testPendingAbandonWithClearRecoveryIsInvalid =
    init
      .then(hookAllocate(Attempt0))
      .then(all {
        phase' = phase.set(Attempt0, PendingAbandon),
        recovery' = recovery,
        ownerAlive' = ownerAlive,
        everRemoved' = everRemoved,
        everWasPendingAbandon' = everWasPendingAbandon.union(Set(Attempt0)),
        pendingAbandonFromDoctor' = pendingAbandonFromDoctor,
        stateTxn' = stateTxn,
        lastDiagnosis' = lastDiagnosis,
        fixState' = RepairAttempted,
      })
      .expect(phase.get(Attempt0) == PendingAbandon)
      .expect(recovery == Clear)
      .expect(adapterHealth(phase, recovery) == Invalid)
      .then(doctorReportResult)
      .expect(fixState == ReportedManual)
      .expect(not(fixState == ReportedFixed))

  run testInvalidTakesPriorityOverBlockedWhenBothConditionsHold =
    init
      .then(hookAllocate(Attempt1))
      .then(all {
        phase' = phase.set(Attempt0, PendingAbandon),
        recovery' = recovery,
        ownerAlive' = ownerAlive,
        everRemoved' = everRemoved,
        everWasPendingAbandon' = everWasPendingAbandon.union(Set(Attempt0)),
        pendingAbandonFromDoctor' = pendingAbandonFromDoctor,
        stateTxn' = stateTxn,
        lastDiagnosis' = lastDiagnosis,
        fixState' = fixState,
      })
      .expect(phase.get(Attempt1) == PendingStart)
      .expect(phase.get(Attempt0) == PendingAbandon)
      .expect(recovery == Clear)
      .expect(adapterHealth(phase, recovery) == Invalid)

  run testRemovedAttemptIsNeverResurrected =
    init
      .then(hookAllocate(Attempt0))
      .then(hookDecideAbandon(Attempt0))
      .then(recoveryProgress(Attempt0))
      .then(completeAbandon(Attempt0))
      .expect(everRemoved.contains(Attempt0))
      .expect(phase.get(Attempt0) == Removed)
      .expect(recovery == Clear)
      .expect(phase.get(Attempt0) != NotAllocated)
      .expect(RemovedAttemptsAreNeverResurrected)
      .expect(Safety)
}
