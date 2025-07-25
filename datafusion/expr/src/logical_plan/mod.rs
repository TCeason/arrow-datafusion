// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

pub mod builder;
mod ddl;
pub mod display;
pub mod dml;
mod extension;
pub(crate) mod invariants;
pub use invariants::{assert_expected_schema, check_subquery_expr, InvariantLevel};
mod plan;
mod statement;
pub mod tree_node;

pub use builder::{
    build_join_schema, requalify_sides_if_needed, table_scan, union,
    wrap_projection_for_join_if_necessary, LogicalPlanBuilder, LogicalPlanBuilderOptions,
    LogicalTableSource, UNNAMED_TABLE,
};
pub use ddl::{
    CreateCatalog, CreateCatalogSchema, CreateExternalTable, CreateFunction,
    CreateFunctionBody, CreateIndex, CreateMemoryTable, CreateView, DdlStatement,
    DropCatalogSchema, DropFunction, DropTable, DropView, OperateFunctionArg,
};
pub use dml::{DmlStatement, WriteOp};
pub use plan::{
    projection_schema, Aggregate, Analyze, ColumnUnnestList, DescribeTable, Distinct,
    DistinctOn, EmptyRelation, Explain, ExplainFormat, ExplainOption, Extension,
    FetchType, Filter, Join, JoinConstraint, JoinType, Limit, LogicalPlan, Partitioning,
    PlanType, Projection, RecursiveQuery, Repartition, SkipType, Sort, StringifiedPlan,
    Subquery, SubqueryAlias, TableScan, ToStringifiedPlan, Union, Unnest, Values, Window,
};
pub use statement::{
    Deallocate, Execute, Prepare, SetVariable, Statement, TransactionAccessMode,
    TransactionConclusion, TransactionEnd, TransactionIsolationLevel, TransactionStart,
};

pub use display::display_schema;

pub use extension::{UserDefinedLogicalNode, UserDefinedLogicalNodeCore};
