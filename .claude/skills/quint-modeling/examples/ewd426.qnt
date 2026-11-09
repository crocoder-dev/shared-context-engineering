module self_stabilization {
  const N: int
  const K: int

  assume _ = K >= N

  val bottom = 0
  val top = N

  var system: int -> int

  pure def has_token(nodes: int -> int, index: int): bool =
    if (index == bottom)
      nodes.get(bottom) == nodes.get(top)
    else
      not(nodes.get(index) == nodes.get(index - 1))

  pure def state_transition(nodes: int -> int, index: int): int =
    if (not(has_token(nodes, index)))
      nodes.get(index)
    else if (index == bottom)
      (nodes.get(bottom) + 1) % K
    else
      nodes.get(index - 1)

  action init = all {
    nondet initial = 0.to(N).setOfMaps(0.to(K - 1)).oneOf()
    system' = initial
  }

  action step = {
    nondet node = 0.to(N).filter(i => has_token(system, i)).oneOf()
    system' = system.set(node, state_transition(system, node))
  }

  action distributed_step = {
    nondet nodes = 0.to(N).filter(i => has_token(system, i)).powerset().exclude(Set()).oneOf()
    system' = nodes.fold(system, (s, x) => s.set(x, state_transition(system, x)))
  }

  pure def count_tokens(nodes: int -> int): int = {
    0.to(N).filter(i => has_token(nodes, i)).size()
  }

  temporal convergence = step.weakFair(Set(system)) implies eventually(count_tokens(system) == 1)
  temporal closure = always(count_tokens(system) == 1 implies always(count_tokens(system) == 1))
  temporal persistence = step.weakFair(Set(system)) implies eventually(always(count_tokens(system) == 1))

  def tokenInv = count_tokens(system) > 0

  pure def show_token(nodes: int -> int): int -> bool =
    nodes.keys().mapBy(i => has_token(nodes, i))
}

module ewd426 {
  import self_stabilization(N = 5, K = 7).*
}

module broken_ewd426 {
  import self_stabilization(N = 3, K = 2).*
}
