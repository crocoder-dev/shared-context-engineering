module basicSpells {
  type Option[a] = Some(a) | None

  pure def require(cond: bool): bool = cond

  run requireTest = all {
    assert(require(4 > 3)),
    assert(not(require(false))),
  }

  pure def requires(cond: bool, error: str): str = {
    if (cond) "" else error
  }

  run requiresTest = all {
    assert(requires(4 > 3, "4 > 3") == ""),
    assert(requires(4 < 3, "false: 4 < 3") == "false: 4 < 3"),
  }


  pure def max(i: int, j: int): int = {
    if (i > j) i else j
  }

  run maxTest = all {
    assert(max(3, 4) == 4),
    assert(max(6, 3) == 6),
    assert(max(10, 10) == 10),
    assert(max(-3, -5) == -3),
    assert(max(-5, -3) == -3),
  }

  pure def min(i: int, j: int): int = {
    if (i < j) i else j
  }

  run minTest = all {
    assert(min(3, 4) == 3),
    assert(min(6, 3) == 3),
    assert(min(10, 10) == 10),
    assert(min(-3, -5) == -5),
    assert(min(-5, -3) == -5),
  }

  pure def abs(i: int): int = {
    if (i < 0) -i else i
  }

  run absTest = all {
    assert(abs(3) == 3),
    assert(abs(-3) == 3),
    assert(abs(0) == 0),
  }

  pure def setRemove(s: Set[a], elem: a): Set[a] = {
    s.exclude(Set(elem))
  }

  run setRemoveTest = all {
    assert(Set(2, 4) == Set(2, 3, 4).setRemove(3)),
    assert(Set() == Set().setRemove(3)),
  }

  pure def setAdd(s: Set[a], elem: a): Set[a] = {
    s.union(Set(elem))
  }

  run setAddTest = all{
    assert(Set(2, 3, 4) == Set(2, 4).setAdd(3)),
    assert(Set(3) == Set().setAdd(3)),
    assert(Set(2,4) == Set(2,4).setAdd(4)),
  }

  pure def has(m: a -> b, key: a): bool = {
    m.keys().contains(key)
  }

  run hasTest = all {
    assert(Map(2 -> 3, 4 -> 5).has(2)),
    assert(not(Map(2 -> 3, 4 -> 5).has(6))),
  }

  pure def getOrElse(m: a -> b, key: a, default: b): b = {
    if (m.has(key)) {
      m.get(key)
    } else {
      default
    }
  }

  run getOrElseTest = all {
    assert(Map(2 -> 3, 4 -> 5).getOrElse(2, 0) == 3),
    assert(Map(2 -> 3, 4 -> 5).getOrElse(7, 11) == 11),
  }

  pure def mapRemove(m: a -> b, key: a): a -> b = {
    m.keys().setRemove(key).mapBy(k => m.get(k))
  }

  run mapRemoveTest = all {
    assert(Map(3 -> 4, 7 -> 8) == Map(3 -> 4, 5 -> 6, 7 -> 8).mapRemove(5)),
    assert(Map() == Map().mapRemove(3)),
  }

  pure def mapRemoveAll(m: a -> b, ks: Set[a]): a -> b = {
      m.keys().exclude(ks).mapBy(k => m.get(k))
  }

  run mapRemoveAllTest =
      val m = Map(3 -> 4, 5 -> 6, 7 -> 8)
      all {
          assert(m.mapRemoveAll(Set(5, 7)) == Map(3 -> 4)),
          assert(m.mapRemoveAll(Set(5, 99999)) == Map(3 -> 4, 7 -> 8)),
      }

  pure def values(m: a -> b): Set[b] = {
    m.keys().map(k => m.get(k))
  }

  run valuesTest = all {
    assert(values(Map()) == Set()),
    assert(values(Map(1 -> 2, 2 -> 3)) == Set(2, 3)),
    assert(values(Map(1 -> 2, 2 -> 3, 3 -> 2)) == Set(2, 3)),
  }

  pure def empty(s: Set[a]): bool = s == Set()

  run emptyTest = all {
    assert(empty(Set()) == true),
    assert(empty(Set(1, 2)) == false),
    assert(empty(Set(Set())) == false),
  }

  pure def sortList(list: List[a], lt: (a, a) => bool): List[a] = {
    pure def insertInOrder(sortedList: List[a], num: a): List[a] = {
      match range(0, sortedList.length()).findFirst(i => not(lt(sortedList[i], num))) {
        | None => sortedList.append(num)
        | Some(index) => sortedList.slice(0, index).append(num).concat(sortedList.slice(index, sortedList.length()))
      }
    }

    list.foldl([], (sortedList, num) => insertInOrder(sortedList, num))
  }

  run listSortedTest = all {
    assert([ 1, 3, 5 ] == sortList([ 5, 1, 3 ], (x, y) => x < y)),
    assert([ 1, 1, 3, 5, 5 ] == sortList([ 5, 1, 3, 1, 5 ], (x, y) => x < y)),
  }

  pure def transformValues(m: a -> b, f: (b) => c): a -> c = {
    m.keys().mapBy(k => f(m.get(k)))
  }

  run transformValuesTest = {
    pure val m = Map("a" -> 1, "b" -> 2)
    assert(m.transformValues(x => x + 1) == Map("a" -> 2, "b" -> 3))
  }

  pure def listMap(l: List[a], f: (a) => b): List[b] = {
    range(0, l.length()).foldl([], (acc, i) => {
      acc.append(f(l[i]))
    })
  }

  run listMapTest = all {
    assert(listMap([1, 2, 3], x => x + 1) == [2, 3, 4]),
    assert(listMap([1, 2, 3], x => x > 1) == [false, true, true]),
  }

  pure def last(v: List[a]): a = {
    v[v.length() - 1]
  }

  run lastTest = all {
    assert(last([1, 2, 3]) == 3),
    assert(last([1]) == 1),
  }

  pure def decreasingRange(start: int, end: int): List[int] = {
    range(end, start + 1).foldl([], (acc, i) => {
      List(i).concat(acc)
    })
  }

  run decreasingRangeTest = all {
    assert(decreasingRange(5, 1) == [5, 4, 3, 2, 1]),
  }

  pure def takeWhile(l: List[a], cond: (a) => bool): List[a] = {
    pure val result = l.foldl(([], true), (acc, e) => {
      if (acc._2 and cond(e)) {
        (acc._1.append(e), true)
      } else {
        (acc._1, false)
      }
    })

    result._1
  }

  run takeWhileTest = all {
    assert(takeWhile([1, 5, 4, 3], (x) => x % 2 == 1) == [1, 5]),
  }

  pure def isPrefixOf(l1: List[a], l2: List[a]): bool = {
    if (l1.length() > l2.length()) {
      false
    } else {
      l1.indices().forall(i => l1[i] == l2[i])
    }
  }

  run isPrefixOfTest = all {
    assert(isPrefixOf([1, 2], [1, 2, 3])),
    assert(not(isPrefixOf([1, 2], [1, 3, 2]))),
    assert(not(isPrefixOf([1, 2], [1]))),
    assert(isPrefixOf([], [1, 2])),
    assert(isPrefixOf([], [])),
  }

  pure def find(s, f) = s.fold(None, (a, i) => if (f(i)) Some(i) else a)

  run findTest = all {
    assert(find(Set(1, 2, 3), x => x == 2) == Some(2)),
    assert(find(Set(1, 2, 3), x => x == 4) == None),
  }

  pure def findFirst(l, f) = l.foldl(None, (a, i) => if (a == None and f(i)) Some(i) else a)

  run findFirstTest = all {
    assert(findFirst([1, 2, 3], x => x > 1) == Some(2)),
    assert(findFirst([1, 2, 3], x => x == 4) == None),
  }

  pure def setByWithDefault(m: a -> b, k: a, op: (b) => b, default: b): a -> b = {
    if (m.has(k))
      m.setBy(k, op)
    else
      m.put(k, default).setBy(k, op)
  }

  run setByWithDefaultTest = all {
    assert(setByWithDefault(Map(1 -> 2, 2 -> 3), 1, x => x + 1, 0) == Map(1 -> 3, 2 -> 3)),
    assert(setByWithDefault(Map(1 -> 2, 2 -> 3), 3, x => x + 1, 0) == Map(1 -> 2, 2 -> 3, 3 -> 1)),
  }

  pure def unwrap(value: Option[a]): a = {
    match value {
      | None => Map().get(value)
      | Some(x) => x
    }
  }

  pure def filterMap(s: Set[a], f: (a) => Option[b]): Set[b] = {
    s.fold(Set(), (acc, e) => {
      match f(e) {
        | Some(x) => acc.union(Set(x))
        | None => acc
      }
    })
  }
}
